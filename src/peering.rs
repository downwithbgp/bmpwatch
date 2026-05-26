use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const PEERING_STATUS_URL: &str = "https://archive.routeviews.org/peers/peering-status.html";
const CACHE_TTL_SECS: u64 = 15 * 60;

/// Fetch peering-status.html over HTTP (blocking, ~10 s timeout).
fn fetch_peering_status() -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .new_agent();
    let mut resp = agent
        .get(PEERING_STATUS_URL)
        .call()
        .map_err(|e| format!("fetch peering status: {e}"))?;
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("read peering status: {e}"))?;
    Ok(body)
}

/// Strip `.routeviews.org` suffix from a collector hostname to get the
/// short key used in Kafka topic names and the bundled TSV.
fn normalize_collector(host: &str) -> String {
    if let Some(s) = host.strip_suffix(".routeviews.org") {
        s.to_string()
    } else {
        host.to_string()
    }
}

/// Parse the plain-text peering-status page into a set of active
/// (collector_key, asn) pairs. Excludes zero-prefix and unparseable rows.
fn parse_peering_status(text: &str) -> HashSet<(String, u32)> {
    let mut active: HashSet<(String, u32)> = HashSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        if lower.contains("collector") && lower.contains("as number") {
            continue;
        }
        if line.starts_with('=') || line.starts_with('-') {
            continue;
        }
        // Format: hostname  ASN  peer_addr  prefixes | CC | registry | ASNAME
        let pipe_parts: Vec<&str> = line.split('|').collect();
        if pipe_parts.is_empty() {
            continue;
        }
        let left: Vec<&str> = pipe_parts[0].split_whitespace().collect();
        if left.len() < 4 {
            continue;
        }
        let hostname = left[0];
        let asn: u32 = match left[1].parse() {
            Ok(a) => a,
            Err(_) => continue,
        };
        let prefixes: u64 = match left[left.len() - 1].parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        if prefixes == 0 {
            continue;
        }
        active.insert((normalize_collector(hostname), asn));
    }
    active
}

fn cache_path() -> PathBuf {
    let base = if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(dir)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache")
    } else {
        PathBuf::from(".")
    };
    base.join("bmpwatch").join("peering_status.txt")
}

fn cache_mtime_secs(path: &PathBuf) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    let age = SystemTime::now().duration_since(mtime).unwrap_or_default();
    Some(age.as_secs())
}

fn read_fresh_cache() -> Option<String> {
    let path = cache_path();
    let age = cache_mtime_secs(&path)?;
    if age >= CACHE_TTL_SECS {
        return None;
    }
    std::fs::read_to_string(&path).ok()
}

fn write_cache(text: &str) {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, text);
}

/// Load the active peering set — the set of (collector_key, asn) pairs
/// that appear in peering-status.html with prefixes > 0.
///
/// Fallback chain:
/// 1. Fresh disk cache (< 15 min)
/// 2. Live fetch from archive.routeviews.org (updates cache on success)
/// 3. Stale disk cache (when fetch fails)
/// 4. Bundled routeviews_peers.tsv (ultimate fallback)
pub(crate) fn load_active_peering_set() -> HashSet<(String, u32)> {
    // 1. Fresh disk cache
    if let Some(cached) = read_fresh_cache() {
        return parse_peering_status(&cached);
    }

    // 2. Live fetch
    match fetch_peering_status() {
        Ok(body) => {
            let active = parse_peering_status(&body);
            write_cache(&body);
            return active;
        }
        Err(e) => {
            eprintln!("bmpwatch: peering status fetch failed: {e}");
        }
    }

    // 3. Stale disk cache (fetch failed but we have old data)
    let path = cache_path();
    if let Ok(cached) = std::fs::read_to_string(&path) {
        eprintln!(
            "bmpwatch: using cached peering status from {}",
            path.display()
        );
        return parse_peering_status(&cached);
    }

    // 4. Bundled TSV fallback
    eprintln!("bmpwatch: no peering status cache, using bundled data");
    crate::dashboard::bundled_active_set()
}

/// Filter a topic list, keeping only topics whose (collector, asn) pair
/// is present in the active peering set. Returns the filtered list and
/// the count of hidden topics.
pub(crate) fn filter_active_topics(
    topics: &[String],
    active: &HashSet<(String, u32)>,
) -> Vec<String> {
    let mut kept = Vec::with_capacity(topics.len());
    let mut hidden: Vec<String> = Vec::new();

    for t in topics {
        let pt = match crate::browser::parse_topic(t) {
            Some(pt) => pt,
            None => {
                hidden.push(t.clone());
                continue;
            }
        };
        let asn: u32 = match pt.asn_str.parse() {
            Ok(a) => a,
            Err(_) => {
                hidden.push(t.clone());
                continue;
            }
        };
        if asn != 0 && active.contains(&(pt.collector.clone(), asn)) {
            kept.push(t.clone());
        } else {
            hidden.push(t.clone());
        }
    }

    if !hidden.is_empty() {
        eprintln!(
            "bmpwatch: hiding {} Kafka stream(s) not in peering status",
            hidden.len()
        );
        if std::env::var("BMPWATCH_DEBUG_PEERING").is_ok() {
            for h in &hidden {
                eprintln!("  hidden: {h}");
            }
        } else {
            eprintln!("  (set BMPWATCH_DEBUG_PEERING=1 to list)");
        }
    }

    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_collector_strips_routeviews_org() {
        assert_eq!(normalize_collector("amsix.ams.routeviews.org"), "amsix.ams");
        assert_eq!(
            normalize_collector("route-views.linx.routeviews.org"),
            "route-views.linx"
        );
        assert_eq!(
            normalize_collector("route-views3.routeviews.org"),
            "route-views3"
        );
    }

    #[test]
    fn test_normalize_collector_passthrough() {
        assert_eq!(normalize_collector("amsix.ams"), "amsix.ams");
        assert_eq!(normalize_collector("some.other.host"), "some.other.host");
    }

    #[test]
    fn test_parse_peering_status_basic() {
        let sample = "\
ROUTEVIEWS COLLECTOR | AS NUMBER | PEERING ADDRESS | PREFIXES | CC | REGION | ASNAME
===========================================
amsix.ams.routeviews.org    29075  80.249.208.27   522 | FR | ripencc | IELO IELO Main Network, FR
amsix.ams.routeviews.org    1103  80.249.208.34  1044754 | NL | ripencc | SURFNET-NL SURFnet, The Netherlands, NL
";
        let active = parse_peering_status(sample);
        assert!(active.contains(&("amsix.ams".to_string(), 29075)));
        assert!(active.contains(&("amsix.ams".to_string(), 1103)));
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn test_parse_excludes_zero_prefixes() {
        let sample = "\
amsix.ams.routeviews.org    29075  80.249.208.27   0 | FR | ripencc | IELO
amsix.ams.routeviews.org    1103  80.249.208.34  1044754 | NL | ripencc | SURFNET-NL
";
        let active = parse_peering_status(sample);
        assert!(!active.contains(&("amsix.ams".to_string(), 29075)));
        assert!(active.contains(&("amsix.ams".to_string(), 1103)));
        assert_eq!(active.len(), 1);
    }

    #[test]
    fn test_parse_skips_headers_and_separators() {
        let sample = "\
ROUTEVIEWS COLLECTOR | AS NUMBER | PEERING ADDRESS | PREFIXES | CC | REGION | ASNAME
===========================================
---
";
        let active = parse_peering_status(sample);
        assert!(active.is_empty());
    }

    #[test]
    fn test_parse_empty_input() {
        let active = parse_peering_status("");
        assert!(active.is_empty());
    }

    #[test]
    fn test_parse_multiple_peers_same_collector_asn() {
        // AS 6777 has two peerings on amsix.ams (different addresses)
        let sample = "\
amsix.ams.routeviews.org    6777  80.249.208.255  104878 | NL | ripencc | AMS-IX-RS, NL
amsix.ams.routeviews.org    6777  80.249.209.0    104847 | NL | ripencc | AMS-IX-RS, NL
";
        let active = parse_peering_status(sample);
        // Both map to the same (collector, asn) pair
        assert_eq!(active.len(), 1);
        assert!(active.contains(&("amsix.ams".to_string(), 6777)));
    }

    #[test]
    fn test_parse_ipv4_ipv6_distinct() {
        let sample = "\
route-views3.routeviews.org    29479  192.0.2.1  952511 | NO | ripencc | Transdata AS
route-views3.routeviews.org    29479  2001:db8::1      2 | NO | ripencc | Transdata AS
";
        let active = parse_peering_status(sample);
        // Both have same (collector, asn) — they collapse in the output set
        assert!(active.contains(&("route-views3".to_string(), 29479)));
        assert_eq!(active.len(), 1);
    }

    #[test]
    fn test_filter_active_topics_keeps_valid() {
        let mut active = HashSet::new();
        active.insert(("amsix.ams".to_string(), 29075));
        active.insert(("route-views.linx".to_string(), 6939));

        let topics = vec![
            "routeviews.amsix.ams.29075.bmp_raw".to_string(),
            "routeviews.route-views.linx.6939.bmp_raw".to_string(),
        ];
        let filtered = filter_active_topics(&topics, &active);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_active_topics_hides_inactive() {
        let mut active = HashSet::new();
        active.insert(("amsix.ams".to_string(), 29075));

        let topics = vec![
            "routeviews.amsix.ams.29075.bmp_raw".to_string(),
            "routeviews.amsix.ams.99999.bmp_raw".to_string(),
        ];
        let filtered = filter_active_topics(&topics, &active);
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].contains("29075"));
    }

    #[test]
    fn test_filter_active_topics_hides_as0() {
        let mut active = HashSet::new();
        active.insert(("bdix".to_string(), 29075)); // some other active peer
        let topics = vec![
            "routeviews.bdix.0.bmp_raw".to_string(),
            "routeviews.bdix.29075.bmp_raw".to_string(),
        ];
        let filtered = filter_active_topics(&topics, &active);
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].contains("29075"));
    }

    #[test]
    fn test_filter_active_topics_hides_unparseable() {
        let mut active = HashSet::new();
        active.insert(("foobar".to_string(), 12345));
        let topics = vec![
            "not-a-valid-topic".to_string(),
            "routeviews.foobar.12345.bmp_raw".to_string(),
        ];
        let filtered = filter_active_topics(&topics, &active);
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].contains("12345"));
    }

    #[test]
    fn test_filter_active_topics_empty_active_set_hides_all() {
        let active: HashSet<(String, u32)> = HashSet::new();
        let topics = vec![
            "routeviews.amsix.ams.29075.bmp_raw".to_string(),
            "routeviews.route-views.linx.6939.bmp_raw".to_string(),
        ];
        let filtered = filter_active_topics(&topics, &active);
        assert_eq!(filtered.len(), 0);
    }
}
