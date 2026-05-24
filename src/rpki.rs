use std::fs;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

/// RPKI validation status for a BGP announcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Valid,
    InvalidWrongAsn,
    InvalidTooLong,
    Invalid,
    NotFound,
}

/// Extra detail for Invalid RPKI results.
#[derive(Debug, Clone)]
pub struct RPKIDetail {
    pub expected_asn: Option<u32>,   // ASN that SHOULD announce this prefix
    pub max_prefix_len: Option<u8>,  // max allowed prefix length from the ROA
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Valid => "VAL",
            Status::InvalidWrongAsn => "ASN",
            Status::InvalidTooLong => "LEN",
            Status::Invalid => "INV",
            Status::NotFound => "NF ",
        }
    }
}

/// A single Validated ROA Payload (prefix + max length + origin ASN).
#[derive(Debug, Clone)]
struct Vrp {
    prefix: u32,
    prefix_len: u8,
    max_len: u8,
    asn: u32,
}

/// In-memory VRP cache with binary search for fast prefix lookup.
pub struct RPKICache {
    vrps: Vec<Vrp>,
    valid_count: u64,
    invalid_count: u64,
    not_found_count: u64,
}

impl RPKICache {
    /// Load RPKI cache from disk, or download from RTR server if stale/missing.
    /// Cache is valid for 6 hours before re-download.
    pub fn load_or_download(host: &str, port: u16) -> Result<Self, String> {
        let cache_path = rpki_cache_path();
        let cache_max_age = Duration::from_secs(6 * 3600);

        // Try loading from disk cache first
        if let Ok(meta) = fs::metadata(&cache_path) {
            if let Ok(mtime) = meta.modified() {
                if let Ok(age) = mtime.elapsed() {
                    if age < cache_max_age {
                        if let Ok(cache) = Self::from_cache_file(&cache_path) {
                            return Ok(cache);
                        }
                    }
                }
            }
        }

        // Download from RTR server
        let cache = Self::from_rtr_server(host, port)?;
        let _ = cache.save_to_file(&cache_path);
        Ok(cache)
    }

    fn from_rtr_server(host: &str, port: u16) -> Result<Self, String> {
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|e| format!("RTR: cannot resolve {host}: {e}"))?
            .next()
            .ok_or_else(|| format!("RTR: no addresses for {host}"))?;
        let mut sock = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
            .map_err(|e| format!("RTR connect to {host}:{port}: {e}"))?;

        sock.set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| format!("RTR read timeout: {e}"))?;

        // Send Reset Query PDU (RFC 8210, Section 5.4):
        // byte 0: Protocol Version = 0
        // byte 1: PDU Type = 2 (Reset Query)
        // bytes 2-3: Session ID = 0
        // bytes 4-7: Length = 8
        let reset_query: [u8; 8] = [0, 2, 0, 0, 0, 0, 0, 8];
        sock.write_all(&reset_query)
            .map_err(|e| format!("RTR write: {e}"))?;

        let mut vrps: Vec<Vrp> = Vec::new();

        // Read PDUs until End of Data (type 7) or Error (type 10)
        loop {
            let mut header = [0u8; 8];
            sock.read_exact(&mut header)
                .map_err(|e| format!("RTR read header: {e}"))?;

            let pdu_type = header[1];
            let length = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;

            if length < 8 {
                return Err(format!("RTR: bad PDU length {length}"));
            }

            let body_len = length - 8;
            let mut body = vec![0u8; body_len];
            if body_len > 0 {
                sock.read_exact(&mut body)
                    .map_err(|e| format!("RTR read body: {e}"))?;
            }

            match pdu_type {
                3 => {
                    // Cache Response — just acknowledge and continue
                }
                4 => {
                    // IPv4 Prefix PDU (RFC 8210, Section 5.6)
                    if body_len >= 12 {
                        let _flags = body[0];
                        let prefix_len = body[1];
                        let max_len = body[2].max(prefix_len);
                        let prefix_bytes = [body[4], body[5], body[6], body[7]];
                        let prefix = u32::from_be_bytes(prefix_bytes);
                        let asn = u32::from_be_bytes([body[8], body[9], body[10], body[11]]);
                        if prefix_len > 0 && prefix_len <= 32 {
                            let mask = if prefix_len == 0 {
                                0
                            } else {
                                !0u32 << (32 - prefix_len)
                            };
                            vrps.push(Vrp {
                                prefix: prefix & mask,
                                prefix_len,
                                max_len,
                                asn,
                            });
                        }
                    }
                }
                6 => {
                    // IPv6 Prefix PDU (RFC 8210, Section 5.7)
                    // We skip IPv6 for now — 99%+ of BGP announcements are IPv4
                }
                7 => {
                    // End of Data — done
                    break;
                }
                10 => {
                    // Error Report — server rejected our query
                    return Err(format!(
                        "RTR error report: {}",
                        std::str::from_utf8(&body).unwrap_or("(binary)")
                    ));
                }
                _ => {} // unknown, skip
            }
        }

        // Sort by prefix for binary search
        vrps.sort_by_key(|v| v.prefix);
        // Deduplicate: keep the most specific (longest prefix_len) for each prefix value
        // Actually, we need to search by coverage, not exact prefix match.
        // For now, just dedup by (prefix, prefix_len, asn) tuple
        vrps.dedup_by(|a, b| a.prefix == b.prefix && a.prefix_len == b.prefix_len && a.asn == b.asn);

        eprintln!(
            "  RPKI: downloaded {} VRPs from {addr}",
            vrps.len()
        );

        Ok(RPKICache {
            vrps,
            valid_count: 0,
            invalid_count: 0,
            not_found_count: 0,
        })
    }

    /// Validate a BGP announcement against the RPKI cache.
    /// Returns (status, detail) — detail has expected ASN for InvalidWrongAsn
    /// and max allowed prefix length for InvalidTooLong.
    pub fn validate(&mut self, prefix_str: &str, asn: u32) -> (Status, RPKIDetail) {
        let status = self.validate_inner(prefix_str, asn);
        let detail = match status {
            Status::InvalidWrongAsn => RPKIDetail {
                expected_asn: self.lookup_authorized_asn(prefix_str),
                max_prefix_len: None,
            },
            Status::InvalidTooLong => RPKIDetail {
                expected_asn: None,
                max_prefix_len: self.lookup_max_len(prefix_str),
            },
            _ => RPKIDetail {
                expected_asn: None,
                max_prefix_len: None,
            },
        };
        (status, detail)
    }

    fn lookup_authorized_asn(&self, prefix_str: &str) -> Option<u32> {
        let (addr, prefix_len) = parse_ipv4_prefix(prefix_str)?;
        let network = addr & if prefix_len == 0 { 0 } else { !0u32 << (32 - prefix_len) };
        for vrp_plen in (0..=prefix_len).rev() {
            let mask = if vrp_plen == 0 { 0 } else { !0u32 << (32 - vrp_plen) };
            let key = network & mask;
            if let Ok(pos) = self.vrps.binary_search_by(|v| v.prefix.cmp(&key)) {
                let mut i = pos;
                while i > 0 && self.vrps[i - 1].prefix == key { i -= 1; }
                while i < self.vrps.len() && self.vrps[i].prefix == key {
                    let v = &self.vrps[i];
                    if v.prefix_len == vrp_plen && v.max_len >= prefix_len {
                        return Some(v.asn);
                    }
                    i += 1;
                }
            }
        }
        None
    }

    fn lookup_max_len(&self, prefix_str: &str) -> Option<u8> {
        let (addr, prefix_len) = parse_ipv4_prefix(prefix_str)?;
        let network = addr & if prefix_len == 0 { 0 } else { !0u32 << (32 - prefix_len) };
        for vrp_plen in (0..=prefix_len).rev() {
            let mask = if vrp_plen == 0 { 0 } else { !0u32 << (32 - vrp_plen) };
            let key = network & mask;
            if let Ok(pos) = self.vrps.binary_search_by(|v| v.prefix.cmp(&key)) {
                let mut i = pos;
                while i > 0 && self.vrps[i - 1].prefix == key { i -= 1; }
                while i < self.vrps.len() && self.vrps[i].prefix == key {
                    let v = &self.vrps[i];
                    if v.prefix_len == vrp_plen {
                        return Some(v.max_len);
                    }
                    i += 1;
                }
            }
        }
        None
    }

    fn validate_inner(&mut self, prefix_str: &str, asn: u32) -> Status {
        let (addr, prefix_len) = match parse_ipv4_prefix(prefix_str) {
            Some(v) => v,
            None => {
                self.not_found_count += 1;
                return Status::NotFound;
            }
        };

        let network = addr & if prefix_len == 0 { 0 } else { !0u32 << (32 - prefix_len) };

        // Find any ROA that covers this prefix. A VRP with prefix P/PL
        // covers announcement N/NL if PL ≤ NL and P matches N's first PL bits.
        for vrp_plen in (0..=prefix_len).rev() {
            let mask = if vrp_plen == 0 { 0 } else { !0u32 << (32 - vrp_plen) };
            let key = network & mask;

            match self.vrps.binary_search_by(|v| v.prefix.cmp(&key)) {
                Ok(pos) => {
                    let mut i = pos;
                    while i > 0 && self.vrps[i - 1].prefix == key {
                        i -= 1;
                    }
                    while i < self.vrps.len() && self.vrps[i].prefix == key {
                        let v = &self.vrps[i];
                        if v.prefix_len == vrp_plen {
                            if v.max_len >= prefix_len {
                                if v.asn == asn {
                                    self.valid_count += 1;
                                    return Status::Valid;
                                } else {
                                    self.invalid_count += 1;
                                    return Status::InvalidWrongAsn;
                                }
                            } else {
                                // ROA covers prefix but max_len < announcement prefix_len
                                self.invalid_count += 1;
                                return Status::InvalidTooLong;
                            }
                        }
                        i += 1;
                    }
                }
                Err(_) => {}
            }
        }

        self.not_found_count += 1;
        Status::NotFound
    }

    pub fn vrp_count(&self) -> usize { self.vrps.len() }
    pub fn valid_count(&self) -> u64 { self.valid_count }
    pub fn invalid_count(&self) -> u64 { self.invalid_count }
    pub fn not_found_count(&self) -> u64 { self.not_found_count }

    fn save_to_file(&self, path: &PathBuf) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("cache dir: {e}"))?;
        }
        let mut buf: Vec<u8> = Vec::with_capacity(4 + self.vrps.len() * 10);
        buf.extend_from_slice(&(self.vrps.len() as u32).to_be_bytes());
        for v in &self.vrps {
            buf.extend_from_slice(&v.prefix.to_be_bytes());
            buf.push(v.prefix_len);
            buf.push(v.max_len);
            buf.extend_from_slice(&v.asn.to_be_bytes());
        }
        fs::write(path, &buf).map_err(|e| format!("cache write: {e}"))?;
        Ok(())
    }

    fn from_cache_file(path: &PathBuf) -> Result<Self, String> {
        let data = fs::read(path).map_err(|e| format!("cache read: {e}"))?;
        if data.len() < 4 {
            return Err("cache too short".into());
        }
        let count = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let mut vrps = Vec::with_capacity(count);
        let mut pos = 4;
        while pos + 10 <= data.len() && vrps.len() < count {
            let prefix = u32::from_be_bytes([data[pos], data[pos+1], data[pos+2], data[pos+3]]);
            let prefix_len = data[pos+4];
            let max_len = data[pos+5];
            let asn = u32::from_be_bytes([data[pos+6], data[pos+7], data[pos+8], data[pos+9]]);
            vrps.push(Vrp { prefix, prefix_len, max_len, asn });
            pos += 10;
        }
        vrps.sort_by_key(|v| v.prefix);
        Ok(RPKICache {
            vrps,
            valid_count: 0,
            invalid_count: 0,
            not_found_count: 0,
        })
    }
}

fn rpki_cache_path() -> PathBuf {
    let base = if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(dir)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache")
    } else {
        PathBuf::from(".")
    };
    base.join("bmpdoctor").join("rpki_cache.bin")
}

/// Parse an IPv4 prefix string like "192.0.2.0/24" → (u32, u8).
fn parse_ipv4_prefix(s: &str) -> Option<(u32, u8)> {
    let (ip_str, len_str) = s.split_once('/')?;
    let prefix_len: u8 = len_str.parse().ok()?;
    if prefix_len > 32 {
        return None;
    }
    let mut octets = ip_str.split('.');
    let a: u8 = octets.next()?.parse().ok()?;
    let b: u8 = octets.next()?.parse().ok()?;
    let c: u8 = octets.next()?.parse().ok()?;
    let d: u8 = octets.next()?.parse().ok()?;
    Some((u32::from_be_bytes([a, b, c, d]), prefix_len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ipv4_prefix() {
        assert_eq!(parse_ipv4_prefix("192.0.2.0/24"), Some((0xC0000200, 24)));
        assert_eq!(parse_ipv4_prefix("10.0.0.0/8"), Some((0x0A000000, 8)));
        assert_eq!(parse_ipv4_prefix("0.0.0.0/0"), Some((0, 0)));
        assert_eq!(parse_ipv4_prefix("not/a/prefix"), None);
        assert_eq!(parse_ipv4_prefix("192.0.2.0/33"), None);
    }

    #[test]
    fn test_validate_simple() {
        let mut cache = RPKICache {
            vrps: vec![
                Vrp { prefix: 0xC0000200, prefix_len: 24, max_len: 24, asn: 64496 },
                Vrp { prefix: 0x0A000000, prefix_len: 8, max_len: 16, asn: 65000 },
            ],
            valid_count: 0,
            invalid_count: 0,
            not_found_count: 0,
        };
        cache.vrps.sort_by_key(|v| v.prefix);

        // Exact match
        assert_eq!(cache.validate("192.0.2.0/24", 64496).0, Status::Valid);
        // Right prefix, wrong ASN
        let (status, detail) = cache.validate("192.0.2.0/24", 65000);
        assert_eq!(status, Status::InvalidWrongAsn);
        assert_eq!(detail.expected_asn, Some(64496));
        // Covered by 10.0.0.0/8 max_len=16, announcing a /12
        assert_eq!(cache.validate("10.0.0.0/12", 65000).0, Status::Valid);
        // Too specific: 10.0.0.0/20 with max_len=16
        let (status, detail) = cache.validate("10.0.0.0/20", 65000);
        assert_eq!(status, Status::InvalidTooLong);
        assert_eq!(detail.max_prefix_len, Some(16));
        // No covering ROA
        assert_eq!(cache.validate("203.0.113.0/24", 65000).0, Status::NotFound);
    }
}
