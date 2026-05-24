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
    pub expected_asn: Option<u32>,
    pub max_prefix_len: Option<u8>,
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

#[derive(Debug, Clone)]
struct Vrp4 {
    prefix: u32,
    prefix_len: u8,
    max_len: u8,
    asn: u32,
}

#[derive(Debug, Clone)]
struct Vrp6 {
    prefix: u128,
    prefix_len: u8,
    max_len: u8,
    asn: u32,
}

pub struct RPKICache {
    vrps4: Vec<Vrp4>,
    vrps6: Vec<Vrp6>,
    valid_count: u64,
    invalid_count: u64,
    not_found_count: u64,
}

impl RPKICache {
    pub fn load_or_download(host: &str, port: u16) -> Result<Self, String> {
        let cache_path = rpki_cache_path();
        let cache_max_age = Duration::from_secs(6 * 3600);

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

        let reset_query: [u8; 8] = [0, 2, 0, 0, 0, 0, 0, 8];
        sock.write_all(&reset_query)
            .map_err(|e| format!("RTR write: {e}"))?;

        let mut vrps4: Vec<Vrp4> = Vec::new();
        let mut vrps6: Vec<Vrp6> = Vec::new();

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
                3 => {} // Cache Response
                4 => {
                    // IPv4 Prefix PDU
                    if body_len >= 12 {
                        let prefix_len = body[1];
                        let max_len = body[2].max(prefix_len);
                        let prefix = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
                        let asn = u32::from_be_bytes([body[8], body[9], body[10], body[11]]);
                        if prefix_len > 0 && prefix_len <= 32 {
                            let mask = if prefix_len == 0 { 0 } else { !0u32 << (32 - prefix_len) };
                            vrps4.push(Vrp4 {
                                prefix: prefix & mask,
                                prefix_len,
                                max_len,
                                asn,
                            });
                        }
                    }
                }
                6 => {
                    // IPv6 Prefix PDU
                    if body_len >= 24 {
                        let prefix_len = body[1];
                        let max_len = body[2].max(prefix_len);
                        let mut prefix_bytes = [0u8; 16];
                        prefix_bytes.copy_from_slice(&body[4..20]);
                        let prefix = u128::from_be_bytes(prefix_bytes);
                        let asn = u32::from_be_bytes([body[20], body[21], body[22], body[23]]);
                        if prefix_len > 0 && prefix_len <= 128 {
                            let mask = if prefix_len == 0 { 0 } else { !0u128 << (128 - prefix_len) };
                            vrps6.push(Vrp6 {
                                prefix: prefix & mask,
                                prefix_len,
                                max_len,
                                asn,
                            });
                        }
                    }
                }
                7 => break,
                10 => {
                    return Err(format!(
                        "RTR error report: {}",
                        std::str::from_utf8(&body).unwrap_or("(binary)")
                    ));
                }
                _ => {}
            }
        }

        vrps4.sort_by_key(|v| v.prefix);
        vrps4.dedup_by(|a, b| a.prefix == b.prefix && a.prefix_len == b.prefix_len && a.asn == b.asn);
        vrps6.sort_by_key(|v| v.prefix);
        vrps6.dedup_by(|a, b| a.prefix == b.prefix && a.prefix_len == b.prefix_len && a.asn == b.asn);

        eprintln!(
            "  RPKI: {} IPv4 + {} IPv6 VRPs from {host}",
            vrps4.len(),
            vrps6.len()
        );

        Ok(RPKICache {
            vrps4,
            vrps6,
            valid_count: 0,
            invalid_count: 0,
            not_found_count: 0,
        })
    }

    pub fn validate(&mut self, prefix_str: &str, asn: u32) -> (Status, RPKIDetail) {
        // Try IPv4 first, then IPv6
        if let Some((addr, prefix_len)) = parse_ipv4_prefix(prefix_str) {
            let status = self.validate_v4(addr, prefix_len, asn);
            let detail = match status {
                Status::InvalidWrongAsn => RPKIDetail {
                    expected_asn: self.lookup_authorized_asn_v4(addr, prefix_len),
                    max_prefix_len: None,
                },
                Status::InvalidTooLong => RPKIDetail {
                    expected_asn: None,
                    max_prefix_len: self.lookup_max_len_v4(addr, prefix_len),
                },
                _ => RPKIDetail { expected_asn: None, max_prefix_len: None },
            };
            return (status, detail);
        }

        if let Some((addr, prefix_len)) = parse_ipv6_prefix(prefix_str) {
            let status = self.validate_v6(addr, prefix_len, asn);
            let detail = match status {
                Status::InvalidWrongAsn => RPKIDetail {
                    expected_asn: self.lookup_authorized_asn_v6(addr, prefix_len),
                    max_prefix_len: None,
                },
                Status::InvalidTooLong => RPKIDetail {
                    expected_asn: None,
                    max_prefix_len: self.lookup_max_len_v6(addr, prefix_len),
                },
                _ => RPKIDetail { expected_asn: None, max_prefix_len: None },
            };
            return (status, detail);
        }

        self.not_found_count += 1;
        (Status::NotFound, RPKIDetail { expected_asn: None, max_prefix_len: None })
    }

    // ── IPv4 validation ──

    fn validate_v4(&mut self, addr: u32, prefix_len: u8, asn: u32) -> Status {
        let network = addr & if prefix_len == 0 { 0 } else { !0u32 << (32 - prefix_len) };
        for vrp_plen in (0..=prefix_len).rev() {
            let mask = if vrp_plen == 0 { 0 } else { !0u32 << (32 - vrp_plen) };
            let key = network & mask;
            match self.vrps4.binary_search_by(|v| v.prefix.cmp(&key)) {
                Ok(pos) => {
                    let mut i = pos;
                    while i > 0 && self.vrps4[i - 1].prefix == key { i -= 1; }
                    while i < self.vrps4.len() && self.vrps4[i].prefix == key {
                        let v = &self.vrps4[i];
                        if v.prefix_len == vrp_plen {
                            if v.max_len >= prefix_len {
                                return if v.asn == asn {
                                    self.valid_count += 1;
                                    Status::Valid
                                } else {
                                    self.invalid_count += 1;
                                    Status::InvalidWrongAsn
                                };
                            } else {
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

    fn lookup_authorized_asn_v4(&self, addr: u32, prefix_len: u8) -> Option<u32> {
        let network = addr & if prefix_len == 0 { 0 } else { !0u32 << (32 - prefix_len) };
        for vrp_plen in (0..=prefix_len).rev() {
            let mask = if vrp_plen == 0 { 0 } else { !0u32 << (32 - vrp_plen) };
            let key = network & mask;
            if let Ok(pos) = self.vrps4.binary_search_by(|v| v.prefix.cmp(&key)) {
                let mut i = pos;
                while i > 0 && self.vrps4[i - 1].prefix == key { i -= 1; }
                while i < self.vrps4.len() && self.vrps4[i].prefix == key {
                    let v = &self.vrps4[i];
                    if v.prefix_len == vrp_plen && v.max_len >= prefix_len {
                        return Some(v.asn);
                    }
                    i += 1;
                }
            }
        }
        None
    }

    fn lookup_max_len_v4(&self, addr: u32, prefix_len: u8) -> Option<u8> {
        let network = addr & if prefix_len == 0 { 0 } else { !0u32 << (32 - prefix_len) };
        for vrp_plen in (0..=prefix_len).rev() {
            let mask = if vrp_plen == 0 { 0 } else { !0u32 << (32 - vrp_plen) };
            let key = network & mask;
            if let Ok(pos) = self.vrps4.binary_search_by(|v| v.prefix.cmp(&key)) {
                let mut i = pos;
                while i > 0 && self.vrps4[i - 1].prefix == key { i -= 1; }
                while i < self.vrps4.len() && self.vrps4[i].prefix == key {
                    let v = &self.vrps4[i];
                    if v.prefix_len == vrp_plen {
                        return Some(v.max_len);
                    }
                    i += 1;
                }
            }
        }
        None
    }

    // ── IPv6 validation ──

    fn validate_v6(&mut self, addr: u128, prefix_len: u8, asn: u32) -> Status {
        let network = addr & if prefix_len == 0 { 0 } else { !0u128 << (128 - prefix_len) };
        for vrp_plen in (0..=prefix_len).rev() {
            let mask = if vrp_plen == 0 { 0 } else { !0u128 << (128 - vrp_plen) };
            let key = network & mask;
            match self.vrps6.binary_search_by(|v| v.prefix.cmp(&key)) {
                Ok(pos) => {
                    let mut i = pos;
                    while i > 0 && self.vrps6[i - 1].prefix == key { i -= 1; }
                    while i < self.vrps6.len() && self.vrps6[i].prefix == key {
                        let v = &self.vrps6[i];
                        if v.prefix_len == vrp_plen {
                            if v.max_len >= prefix_len {
                                return if v.asn == asn {
                                    self.valid_count += 1;
                                    Status::Valid
                                } else {
                                    self.invalid_count += 1;
                                    Status::InvalidWrongAsn
                                };
                            } else {
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

    fn lookup_authorized_asn_v6(&self, addr: u128, prefix_len: u8) -> Option<u32> {
        let network = addr & if prefix_len == 0 { 0 } else { !0u128 << (128 - prefix_len) };
        for vrp_plen in (0..=prefix_len).rev() {
            let mask = if vrp_plen == 0 { 0 } else { !0u128 << (128 - vrp_plen) };
            let key = network & mask;
            if let Ok(pos) = self.vrps6.binary_search_by(|v| v.prefix.cmp(&key)) {
                let mut i = pos;
                while i > 0 && self.vrps6[i - 1].prefix == key { i -= 1; }
                while i < self.vrps6.len() && self.vrps6[i].prefix == key {
                    let v = &self.vrps6[i];
                    if v.prefix_len == vrp_plen && v.max_len >= prefix_len {
                        return Some(v.asn);
                    }
                    i += 1;
                }
            }
        }
        None
    }

    fn lookup_max_len_v6(&self, addr: u128, prefix_len: u8) -> Option<u8> {
        let network = addr & if prefix_len == 0 { 0 } else { !0u128 << (128 - prefix_len) };
        for vrp_plen in (0..=prefix_len).rev() {
            let mask = if vrp_plen == 0 { 0 } else { !0u128 << (128 - vrp_plen) };
            let key = network & mask;
            if let Ok(pos) = self.vrps6.binary_search_by(|v| v.prefix.cmp(&key)) {
                let mut i = pos;
                while i > 0 && self.vrps6[i - 1].prefix == key { i -= 1; }
                while i < self.vrps6.len() && self.vrps6[i].prefix == key {
                    let v = &self.vrps6[i];
                    if v.prefix_len == vrp_plen {
                        return Some(v.max_len);
                    }
                    i += 1;
                }
            }
        }
        None
    }

    pub fn vrp_count(&self) -> usize { self.vrps4.len() + self.vrps6.len() }
    pub fn valid_count(&self) -> u64 { self.valid_count }
    pub fn invalid_count(&self) -> u64 { self.invalid_count }
    pub fn not_found_count(&self) -> u64 { self.not_found_count }

    // ── Cache serialization ──

    fn save_to_file(&self, path: &PathBuf) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("cache dir: {e}"))?;
        }
        // Format: 4-byte v4 count, v4 entries (10 bytes each),
        //         4-byte v6 count, v6 entries (22 bytes each)
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&(self.vrps4.len() as u32).to_be_bytes());
        for v in &self.vrps4 {
            buf.extend_from_slice(&v.prefix.to_be_bytes());
            buf.push(v.prefix_len);
            buf.push(v.max_len);
            buf.extend_from_slice(&v.asn.to_be_bytes());
        }
        buf.extend_from_slice(&(self.vrps6.len() as u32).to_be_bytes());
        for v in &self.vrps6 {
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
        if data.len() < 8 {
            return Err("cache too short".into());
        }

        let count4 = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let mut vrps4 = Vec::with_capacity(count4);
        let mut pos = 4;
        while pos + 10 <= data.len() && vrps4.len() < count4 {
            let prefix = u32::from_be_bytes([data[pos], data[pos+1], data[pos+2], data[pos+3]]);
            let prefix_len = data[pos+4];
            let max_len = data[pos+5];
            let asn = u32::from_be_bytes([data[pos+6], data[pos+7], data[pos+8], data[pos+9]]);
            vrps4.push(Vrp4 { prefix, prefix_len, max_len, asn });
            pos += 10;
        }

        let mut vrps6 = Vec::new();
        if pos + 4 <= data.len() {
            let count6 = u32::from_be_bytes([data[pos], data[pos+1], data[pos+2], data[pos+3]]) as usize;
            pos += 4;
            vrps6 = Vec::with_capacity(count6);
            while pos + 22 <= data.len() && vrps6.len() < count6 {
                let mut prefix_bytes = [0u8; 16];
                prefix_bytes.copy_from_slice(&data[pos..pos+16]);
                let prefix = u128::from_be_bytes(prefix_bytes);
                let prefix_len = data[pos+16];
                let max_len = data[pos+17];
                let asn = u32::from_be_bytes([data[pos+18], data[pos+19], data[pos+20], data[pos+21]]);
                vrps6.push(Vrp6 { prefix, prefix_len, max_len, asn });
                pos += 22;
            }
        }

        vrps4.sort_by_key(|v| v.prefix);
        vrps6.sort_by_key(|v| v.prefix);
        Ok(RPKICache {
            vrps4,
            vrps6,
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
    base.join("bmpwatch").join("rpki_cache.bin")
}

fn parse_ipv4_prefix(s: &str) -> Option<(u32, u8)> {
    let (ip_str, len_str) = s.split_once('/')?;
    let prefix_len: u8 = len_str.parse().ok()?;
    if prefix_len > 32 { return None; }
    let mut octets = ip_str.split('.');
    let a: u8 = octets.next()?.parse().ok()?;
    let b: u8 = octets.next()?.parse().ok()?;
    let c: u8 = octets.next()?.parse().ok()?;
    let d: u8 = octets.next()?.parse().ok()?;
    // Verify no extra octets
    if octets.next().is_some() { return None; }
    Some((u32::from_be_bytes([a, b, c, d]), prefix_len))
}

fn parse_ipv6_prefix(s: &str) -> Option<(u128, u8)> {
    let (ip_str, len_str) = s.split_once('/')?;
    let prefix_len: u8 = len_str.parse().ok()?;
    if prefix_len > 128 { return None; }
    // Use the standard library's IPv6 parser
    let addr: std::net::Ipv6Addr = ip_str.parse().ok()?;
    Some((u128::from_be_bytes(addr.octets()), prefix_len))
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
        // Extra octet rejected
        assert_eq!(parse_ipv4_prefix("1.2.3.4.5/24"), None);
    }

    #[test]
    fn test_parse_ipv6_prefix() {
        assert!(parse_ipv6_prefix("2001:db8::/32").is_some());
        assert!(parse_ipv6_prefix("::1/128").is_some());
        assert!(parse_ipv6_prefix("::/0").is_some());
        assert_eq!(parse_ipv6_prefix("not/an/ipv6"), None);
        assert_eq!(parse_ipv6_prefix("2001:db8::/129"), None);
    }

    #[test]
    fn test_validate_ipv6() {
        let mut cache = RPKICache {
            vrps4: vec![],
            vrps6: vec![
                Vrp6 { prefix: 0x20010DB8000000000000000000000000, prefix_len: 32, max_len: 48, asn: 65000 },
            ],
            valid_count: 0, invalid_count: 0, not_found_count: 0,
        };
        cache.vrps6.sort_by_key(|v| v.prefix);

        // Valid: right prefix, right ASN, within max_len
        assert_eq!(cache.validate("2001:db8::/32", 65000).0, Status::Valid);
        assert_eq!(cache.validate("2001:db8:1234::/48", 65000).0, Status::Valid);
        // Wrong ASN
        assert_eq!(cache.validate("2001:db8::/32", 64496).0, Status::InvalidWrongAsn);
        // Too specific
        assert_eq!(cache.validate("2001:db8:1234:5678::/64", 65000).0, Status::InvalidTooLong);
        // No covering ROA
        assert_eq!(cache.validate("2001:db9::/32", 65000).0, Status::NotFound);
    }

    #[test]
    fn test_validate_v4_still_works() {
        let mut cache = RPKICache {
            vrps4: vec![
                Vrp4 { prefix: 0xC0000200, prefix_len: 24, max_len: 24, asn: 64496 },
                Vrp4 { prefix: 0x0A000000, prefix_len: 8, max_len: 16, asn: 65000 },
            ],
            vrps6: vec![],
            valid_count: 0, invalid_count: 0, not_found_count: 0,
        };
        cache.vrps4.sort_by_key(|v| v.prefix);

        assert_eq!(cache.validate("192.0.2.0/24", 64496).0, Status::Valid);
        let (s, d) = cache.validate("192.0.2.0/24", 65000);
        assert_eq!(s, Status::InvalidWrongAsn);
        assert_eq!(d.expected_asn, Some(64496));
    }
}
