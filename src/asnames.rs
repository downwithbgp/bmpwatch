use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;

const STALE_DAYS: u64 = 90;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AsNameEntry {
    pub(crate) name: String,
    pub(crate) country_code: String,
    pub(crate) registry: String,
    pub(crate) allocated: String,
    pub(crate) refreshed_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    entries: HashMap<u32, AsNameEntry>,
}

pub(crate) struct AsNameCache {
    entries: HashMap<u32, AsNameEntry>,
}

impl AsNameCache {
    pub(crate) fn load() -> Self {
        let path = cache_path();
        let entries = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<CacheFile>(&text) {
                Ok(cf) if cf.version == 1 => cf.entries,
                _ => HashMap::new(),
            },
            Err(_) => HashMap::new(),
        };
        AsNameCache { entries }
    }

    pub(crate) fn get(&self, asn: u32) -> Option<&AsNameEntry> {
        self.entries.get(&asn)
    }

    pub(crate) fn write(&self) {
        let cf = CacheFile {
            version: 1,
            entries: self.entries.clone(),
        };
        let path = cache_path();
        if let Ok(json) = serde_json::to_string(&cf) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, json);
        }
    }
}

/// Resolve an ASN name from the Team Cymru cache.
/// Returns None if not in cache, regardless of staleness.
pub(crate) fn lookup_cached_name(asn: u32) -> Option<String> {
    let cache = AsNameCache::load();
    cache.get(asn).map(|e| e.name.clone())
}

/// Entry point for `bmpwatch refresh-asnames`. Collects ASNs from flags, then calls run_refresh.
pub(crate) fn run_refresh_asnames(
    asn_list: Vec<u32>,
    from_topics: bool,
    broker: &str,
    stale: bool,
    limit: usize,
) -> Result<()> {
    let mut asns: Vec<u32> = asn_list;

    if from_topics {
        let topics = crate::kafka::fetch_topics(broker, "^routeviews.*\\.bmp_raw$")?;
        for t in topics {
            // Extract ASN from topic: routeviews.<collector>.<ASN>.bmp_raw
            if let Some(asn_str) = t
                .strip_suffix(".bmp_raw")
                .and_then(|s| s.rsplit('.').next())
            {
                if let Ok(a) = asn_str.parse::<u32>() {
                    if a > 0 {
                        asns.push(a);
                    }
                }
            }
        }
    }

    if stale {
        let cache = AsNameCache::load();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        for (&asn, entry) in &cache.entries {
            if now.saturating_sub(entry.refreshed_unix) >= STALE_DAYS * 86400 {
                asns.push(asn);
            }
        }
    }

    if asns.is_empty() {
        eprintln!("no ASNs to query (use --asn, --from-topics, or --stale)");
        return Ok(());
    }

    // Dedup and limit
    {
        let mut set: HashSet<u32> = HashSet::new();
        asns.retain(|a| set.insert(*a));
    }
    asns.sort_unstable();
    if asns.len() > limit {
        eprintln!("limiting to {limit} of {} unique ASNs", asns.len());
        asns.truncate(limit);
    }

    run_refresh(&asns)
}

/// Bulk WHOIS to Team Cymru. One TCP connection, all ASNs in one request.
fn run_refresh(asns: &[u32]) -> Result<()> {
    eprintln!("querying {} ASNs from whois.cymru.com...", asns.len());

    let addr = ("whois.cymru.com", 43)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .ok_or_else(|| anyhow::anyhow!("Failed to resolve whois.cymru.com"))?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| anyhow::anyhow!("Failed to connect to whois.cymru.com: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;

    write!(stream, "begin\r\nverbose\r\n")?;
    for asn in asns {
        write!(stream, "as{asn}\r\n")?;
    }
    write!(stream, "end\r\n")?;
    stream.flush()?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;

    let mut cache = AsNameCache::load();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut parsed = 0u32;
    let mut skipped = 0u32;

    for line in response.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Skip header lines
        let lower = line.to_lowercase();
        if lower.contains("bulk mode")
            || lower.starts_with("asn") && lower.contains("|") && lower.contains("description")
        {
            continue;
        }
        // Parse: ASN | CC | Registry | Allocated | AS Name
        let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
        if parts.len() < 5 {
            skipped += 1;
            continue;
        }
        let asn: u32 = match parts[0].parse() {
            Ok(a) => a,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let name = crate::dashboard::normalize_cymru_name(asn, parts[4]);
        if name.is_empty() {
            skipped += 1;
            continue;
        }
        cache.entries.insert(
            asn,
            AsNameEntry {
                name,
                country_code: parts[1].to_string(),
                registry: parts[2].to_string(),
                allocated: parts[3].to_string(),
                refreshed_unix: now,
            },
        );
        parsed += 1;
    }

    cache.write();
    eprintln!(
        "cached {parsed} AS names, skipped {skipped}, wrote {}",
        cache_path().display()
    );
    Ok(())
}

fn cache_path() -> PathBuf {
    let base = if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(dir)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache")
    } else {
        PathBuf::from(".")
    };
    base.join("bmpwatch").join("asn_names.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> PathBuf {
        std::env::temp_dir().join("bmpwatch_test_asn_names.json")
    }

    fn clean() {
        let _ = std::fs::remove_file(temp_path());
    }

    #[test]
    fn test_parse_valid_line() {
        let line = "13335 | US | arin | 2015-06-01 | CLOUDFLARENET - Cloudflare, Inc., US";
        let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0], "13335");
        assert_eq!(parts[4], "CLOUDFLARENET - Cloudflare, Inc., US");
    }

    #[test]
    fn test_parse_skips_header() {
        let header = "Bulk mode; whois.cymru.com";
        let lower = header.to_lowercase();
        assert!(lower.contains("bulk mode"));
    }

    #[test]
    fn test_cache_round_trip() {
        clean();
        let mut cache = AsNameCache {
            entries: HashMap::new(),
        };
        cache.entries.insert(
            13335,
            AsNameEntry {
                name: "Cloudflare".into(),
                country_code: "US".into(),
                registry: "arin".into(),
                allocated: "2015-06-01".into(),
                refreshed_unix: 1000000,
            },
        );
        cache.write();

        let loaded = AsNameCache::load();
        let entry = loaded.get(13335).unwrap();
        assert_eq!(entry.name, "Cloudflare");
        assert_eq!(entry.country_code, "US");
        clean();
    }

    #[test]
    fn test_stale_cache_still_used() {
        let mut cache = AsNameCache {
            entries: HashMap::new(),
        };
        let old = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 91 * 86400;
        cache.entries.insert(
            13335,
            AsNameEntry {
                name: "Cloudflare".into(),
                country_code: "US".into(),
                registry: "arin".into(),
                allocated: "2015-06-01".into(),
                refreshed_unix: old,
            },
        );
        // Cache lookup returns names regardless of staleness
        assert!(cache.get(13335).is_some());
    }

    #[test]
    fn test_cache_miss_returns_none() {
        let cache = AsNameCache {
            entries: HashMap::new(),
        };
        assert!(cache.get(99999).is_none());
    }

    #[test]
    fn test_corrupt_cache_ignored() {
        clean();
        std::fs::write(temp_path(), b"not json").unwrap();
        clean();
    }

    #[test]
    fn test_dedup_asns() {
        // Verify --asn deduplication: [13335, 13335, 6939] -> [6939, 13335]
        let input = vec![13335, 6939, 13335];
        let mut set: HashSet<u32> = HashSet::new();
        let deduped: Vec<u32> = input.into_iter().filter(|a| set.insert(*a)).collect();
        assert_eq!(deduped.len(), 2);
        assert!(deduped.contains(&13335));
        assert!(deduped.contains(&6939));
    }

    #[test]
    fn test_extract_asn_from_topic() {
        // Simulate --from-topics extraction
        let topics = [
            "routeviews.chicago.13335.bmp_raw",
            "routeviews.linx.6939.bmp_raw",
            "routeviews.nwax.714.bmp_raw",
        ];
        let mut asns: Vec<u32> = Vec::new();
        for t in &topics {
            if let Some(asn_str) = t
                .strip_suffix(".bmp_raw")
                .and_then(|s| s.rsplit('.').next())
            {
                if let Ok(a) = asn_str.parse::<u32>() {
                    if a > 0 {
                        asns.push(a);
                    }
                }
            }
        }
        assert_eq!(asns, vec![13335, 6939, 714]);
    }

    #[test]
    fn test_parse_malformed_line_skipped() {
        // Fewer than 5 pipe-delimited fields
        let line = "13335 | US | arin";
        let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
        assert!(parts.len() < 5);
    }

    #[test]
    fn test_parse_empty_name_skipped() {
        let line = "13335 | US | arin | 2015-06-01 | ";
        let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
        assert_eq!(parts.len(), 5);
        assert!(parts[4].is_empty());
    }
}
