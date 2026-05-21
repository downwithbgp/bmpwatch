use crate::state::{Finding, Severity};

// Lint rule names. Each rule is a stable snake_case identifier used in
// machine-oriented lint output. Rules map to RFC 7854 requirements:
//
//   invalid_bmp_version          — RFC 7854, Section 9: version must be 3
//   truncated_frame              — RFC 7854, Section 3.1: declared length
//                                  exceeds available data
//   unknown_bmp_type             — RFC 7854, Section 9 / IANA BMP Parameters:
//                                  message type outside registered range
//   parse_error                  — bgpkit-parser could not decode the frame
//   route_monitoring_before_peer_up — RFC 7854, Section 3.3: RM messages
//                                     should follow Peer Up for that peer
//   duplicate_peer_up            — RFC 7854, Section 3.6: at most one active
//                                  Peer Up per peer at a time
//   peer_down_without_peer_up    — RFC 7854, Section 3.5: Peer Down should
//                                  follow a corresponding Peer Up
//   timestamp_regression         — RFC 7854, Section 3.2: timestamps should
//                                  be monotonically non-decreasing per peer
//

pub const RULE_INVALID_BMP_VERSION: &str = "invalid_bmp_version";
pub const RULE_TRUNCATED_FRAME: &str = "truncated_frame";
pub const RULE_UNKNOWN_BMP_TYPE: &str = "unknown_bmp_type";
pub const RULE_PARSE_ERROR: &str = "parse_error";
pub const RULE_ROUTE_MONITORING_BEFORE_PEER_UP: &str = "route_monitoring_before_peer_up";
pub const RULE_DUPLICATE_PEER_UP: &str = "duplicate_peer_up";
pub const RULE_PEER_DOWN_WITHOUT_PEER_UP: &str = "peer_down_without_peer_up";
pub const RULE_TIMESTAMP_REGRESSION: &str = "timestamp_regression";

pub fn finding_invalid_version(offset: u64, version: u8) -> Finding {
    Finding {
        severity: Severity::Error,
        rule: RULE_INVALID_BMP_VERSION.to_string(),
        offset: Some(offset),
        peer: None,
        message: format!(
            "Unsupported BMP version {} at offset {} (expected 3)",
            version, offset
        ),
    }
}

pub fn finding_unknown_type(offset: u64, msg_type_raw: u8) -> Finding {
    Finding {
        severity: Severity::Warn,
        rule: RULE_UNKNOWN_BMP_TYPE.to_string(),
        offset: Some(offset),
        peer: None,
        message: format!(
            "Unknown BMP message type {} at offset {}",
            msg_type_raw, offset
        ),
    }
}

pub fn finding_parse_error(
    offset: u64,
    peer: Option<crate::state::PeerKey>,
    detail: String,
) -> Finding {
    Finding {
        severity: Severity::Error,
        rule: RULE_PARSE_ERROR.to_string(),
        offset: Some(offset),
        peer,
        message: format!("Parse error at offset {}: {}", offset, detail),
    }
}

pub fn finding_route_monitoring_before_peer_up(
    offset: u64,
    peer: crate::state::PeerKey,
) -> Finding {
    Finding {
        severity: Severity::Warn,
        rule: RULE_ROUTE_MONITORING_BEFORE_PEER_UP.to_string(),
        offset: Some(offset),
        peer: Some(peer.clone()),
        message: format!(
            "Route monitoring message for peer {} at offset {} before any Peer Up notification",
            peer.display(),
            offset
        ),
    }
}

pub fn finding_duplicate_peer_up(offset: u64, peer: crate::state::PeerKey) -> Finding {
    Finding {
        severity: Severity::Warn,
        rule: RULE_DUPLICATE_PEER_UP.to_string(),
        offset: Some(offset),
        peer: Some(peer.clone()),
        message: format!(
            "Duplicate Peer Up notification for peer {} at offset {} (already active)",
            peer.display(),
            offset
        ),
    }
}

pub fn finding_peer_down_without_peer_up(
    offset: u64,
    peer: crate::state::PeerKey,
    reason: u8,
) -> Finding {
    Finding {
        severity: Severity::Warn,
        rule: RULE_PEER_DOWN_WITHOUT_PEER_UP.to_string(),
        offset: Some(offset),
        peer: Some(peer.clone()),
        message: format!(
            "Peer Down notification (reason {}) for peer {} at offset {} without prior Peer Up",
            reason,
            peer.display(),
            offset
        ),
    }
}

pub fn finding_timestamp_regression(
    offset: u64,
    peer: crate::state::PeerKey,
    prev_secs: u32,
    prev_us: u32,
    curr_secs: u32,
    curr_us: u32,
) -> Finding {
    Finding {
        severity: Severity::Warn,
        rule: RULE_TIMESTAMP_REGRESSION.to_string(),
        offset: Some(offset),
        peer: Some(peer.clone()),
        message: format!(
            "Timestamp regression for peer {} at offset {}: previous {}.{:06}s, current {}.{:06}s",
            peer.display(),
            offset,
            prev_secs,
            prev_us,
            curr_secs,
            curr_us,
        ),
    }
}
