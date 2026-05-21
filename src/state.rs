use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct PeerKey {
    pub peer_asn: Option<u32>,
    pub peer_ip: Option<String>,
    pub peer_distinguisher: Option<String>,
}

impl PeerKey {
    pub fn display(&self) -> String {
        match (&self.peer_ip, self.peer_asn) {
            (Some(ip), Some(asn)) => format!("AS{asn} {ip}"),
            (Some(ip), None) => ip.clone(),
            (None, Some(asn)) => format!("AS{asn}"),
            (None, None) => "unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Severity {
    Info,
    Warn,
    Error,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warn => "WARN",
            Severity::Error => "ERR",
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Severity::Info => 0,
            Severity::Warn => 1,
            Severity::Error => 2,
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub rule: String,
    pub offset: Option<u64>,
    pub peer: Option<PeerKey>,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct PeerState {
    pub peer_up_seen: bool,
    pub active: bool,
    pub peer_up_count: u64,
    pub peer_down_count: u64,
    pub route_monitoring_count: u64,
    pub other_message_count: u64,
    pub last_timestamp: Option<(u32, u32)>,
    pub first_timestamp: Option<(u32, u32)>,
    pub update_before_peer_up_count: u64,
    pub timestamp_regression_count: u64,
    pub last_peer_down_reason: Option<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct DoctorState {
    pub file_path: String,
    pub file_size: u64,
    pub format: String,
    pub total_messages: u64,
    pub malformed_messages: u64,
    pub by_type: BTreeMap<u8, u64>,
    pub peers: BTreeMap<PeerKey, PeerState>,
    pub findings: Vec<Finding>,
    pub bgp_elem_count: u64,
}
