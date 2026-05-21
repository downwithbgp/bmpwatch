use serde::Serialize;

use crate::raw_bmp::BmpMessageType;
use crate::state::{Finding, PeerKey};

#[derive(Debug, Clone, Serialize)]
pub struct JsonlEvent {
    pub offset: u64,
    pub bmp_type: Option<String>,
    pub bmp_type_id: u8,
    pub peer_asn: Option<u32>,
    pub peer_ip: Option<String>,
    pub peer_distinguisher: Option<String>,
    pub timestamp_seconds: Option<u32>,
    pub timestamp_microseconds: Option<u32>,
    pub message_length: u32,
    pub parse_status: String,
    pub bgp_elems_count: u64,
    pub findings: Vec<JsonlFinding>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonlFinding {
    pub severity: String,
    pub rule: String,
    pub message: String,
}

impl JsonlEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn from_frame(
        offset: u64,
        msg_type: Option<BmpMessageType>,
        msg_type_raw: u8,
        peer: Option<&PeerKey>,
        timestamp: Option<(u32, u32)>,
        msg_len: u32,
        parse_status: &str,
        bgp_elems_count: u64,
        findings: &[Finding],
    ) -> Self {
        let (ts_sec, ts_us) = timestamp.unwrap_or((0, 0));
        JsonlEvent {
            offset,
            bmp_type: msg_type.map(|t| t.as_str().to_string()),
            bmp_type_id: msg_type_raw,
            peer_asn: peer.and_then(|p| p.peer_asn),
            peer_ip: peer.and_then(|p| p.peer_ip.clone()),
            peer_distinguisher: peer.and_then(|p| p.peer_distinguisher.clone()),
            timestamp_seconds: timestamp.map(|_| ts_sec),
            timestamp_microseconds: timestamp.map(|_| ts_us),
            message_length: msg_len,
            parse_status: parse_status.to_string(),
            bgp_elems_count,
            findings: findings
                .iter()
                .map(|f| JsonlFinding {
                    severity: f.severity.as_str().to_string(),
                    rule: f.rule.clone(),
                    message: f.message.clone(),
                })
                .collect(),
        }
    }
}

pub fn emit_jsonl(events: &[JsonlEvent]) {
    for event in events {
        let line = serde_json::to_string(event)
            .unwrap_or_else(|e| format!(r#"{{"error":"serialization failed: {e}"}}"#));
        println!("{line}");
    }
}

pub fn max_exit_code(findings: &[Finding]) -> i32 {
    let mut max = 0;
    for f in findings {
        let code = f.severity.exit_code();
        if code > max {
            max = code;
        }
    }
    max
}
