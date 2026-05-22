use crate::raw_bmp::BmpMessageType;
use crate::state::{DoctorState, Finding, Severity};
use serde::Serialize;

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit_idx])
    }
}

pub fn render_inspect(state: &DoctorState, truncated: bool) {
    println!("=== BMPDoctor Inspect ===");
    println!();
    println!("File:        {}", state.file_path);
    println!("Format:      {}", state.format);
    println!("Size:        {}", format_size(state.file_size));
    println!("Messages:    {}", state.total_messages);

    if state.malformed_messages > 0 {
        println!(
            "Malformed:   {} (frame-level errors)",
            state.malformed_messages
        );
    }

    if state.bgp_elem_count > 0 {
        println!("BGP Elems:   {}", state.bgp_elem_count);
    }

    println!();
    println!("Message counts by type:");

    let types: Vec<(&u8, &u64)> = {
        let mut v: Vec<_> = state.by_type.iter().collect();
        v.sort_by_key(|(k, _)| *k);
        v
    };

    for (type_id, count) in &types {
        let type_name = BmpMessageType::from_u8(**type_id)
            .map(|t| t.as_str().to_string())
            .unwrap_or_else(|| format!("Unknown({type_id})"));
        println!("  {type_name:<30} {count}");
    }

    if !state.peers.is_empty() {
        println!();
        println!("Peers observed: {}", state.peers.len());

        let active_count = state.peers.values().filter(|p| p.active).count();
        println!("Active at end:  {}", active_count);

        println!();
        println!("Top peers by route-monitoring messages:");

        let mut peer_list: Vec<_> = state.peers.iter().collect();
        peer_list.sort_by_key(|(_, ps)| std::cmp::Reverse(ps.route_monitoring_count));
        let top_n = peer_list.len().min(10);
        for (i, (pk, ps)) in peer_list.iter().take(top_n).enumerate() {
            let status = if ps.active { "active" } else { "inactive" };
            println!(
                "  {}. {:30} {:>8} updates  [{status}]",
                i + 1,
                pk.display(),
                ps.route_monitoring_count,
            );
        }
    }

    println!();
    println!("Findings summary:");

    let info_count = state
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Info)
        .count();
    let warn_count = state
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Warn)
        .count();
    let err_count = state
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();

    println!("  INFO:  {info_count}");
    println!("  WARN:  {warn_count}");
    println!("  ERROR: {err_count}");

    if truncated {
        println!(
            "  NOTE: findings truncated at {} ({} dropped). Use --max-findings to raise the cap.",
            state.findings.len(),
            state.findings_dropped,
        );
    }

    if !state.findings.is_empty() {
        println!();
        println!("Findings detail (first 20):");
        for (i, f) in state.findings.iter().take(20).enumerate() {
            let peer_str = f
                .peer
                .as_ref()
                .map(|p| format!(" peer={}", p.display()))
                .unwrap_or_default();
            let offset_str = f.offset.map(|o| format!(" offset={o}")).unwrap_or_default();
            println!(
                "  {}. {} {}{}{}: {}",
                i + 1,
                f.severity,
                f.rule,
                offset_str,
                peer_str,
                f.message,
            );
        }
        if state.findings.len() > 20 {
            println!("  ... and {} more", state.findings.len() - 20);
        }
    }
}

pub fn render_lint(findings: &[Finding], truncated: bool, dropped: u64) {
    for f in findings {
        let peer_str = f
            .peer
            .as_ref()
            .map(|p| format!(" peer={}", p.display()))
            .unwrap_or_default();
        let offset_str = f.offset.map(|o| format!(" offset={o}")).unwrap_or_default();
        println!(
            "{} {}{}{} {}",
            f.severity, f.rule, offset_str, peer_str, f.message
        );
    }
    if truncated {
        eprintln!(
            "NOTE: findings truncated at {} ({} dropped). Use --max-findings to raise the cap.",
            findings.len(),
            dropped,
        );
    }
}

#[derive(Serialize)]
struct InspectSummary<'a> {
    file: &'a str,
    format: &'a str,
    size_bytes: u64,
    total_messages: u64,
    malformed_messages: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    bgp_elem_count: Option<u64>,
    by_type: std::collections::BTreeMap<String, u64>,
    peers_observed: usize,
    active_peers: usize,
    info_count: usize,
    warn_count: usize,
    error_count: usize,
    findings_truncated: bool,
    findings_dropped_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    container: Option<ContainerSummary>,
}

#[derive(Serialize)]
struct ContainerSummary {
    container_records: u64,
    #[serde(skip_serializing_if = "is_zero")]
    raw_bmp_payloads: u64,
    #[serde(skip_serializing_if = "is_zero")]
    openbmp_wrapped_payloads: u64,
    #[serde(skip_serializing_if = "is_zero")]
    unrecognized_payloads: u64,
    #[serde(skip_serializing_if = "is_zero")]
    openbmp_unwrap_errors: u64,
    #[serde(skip_serializing_if = "is_zero")]
    inner_bmp_parse_errors: u64,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

pub fn render_inspect_json(state: &DoctorState, truncated: bool) {
    let by_type: std::collections::BTreeMap<String, u64> = state
        .by_type
        .iter()
        .map(|(id, count)| {
            let name = BmpMessageType::from_u8(*id)
                .map(|t| t.as_str().to_string())
                .unwrap_or_else(|| format!("Unknown({id})"));
            (name, *count)
        })
        .collect();

    let active_peers = state.peers.values().filter(|p| p.active).count();

    let info_count = state
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Info)
        .count();
    let warn_count = state
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Warn)
        .count();
    let error_count = state
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();

    let bgp_elems = if state.bgp_elem_count > 0 {
        Some(state.bgp_elem_count)
    } else {
        None
    };

    let container = if state.container_stats.has_data() {
        Some(ContainerSummary {
            container_records: state.container_stats.container_records,
            raw_bmp_payloads: state.container_stats.raw_bmp_payloads,
            openbmp_wrapped_payloads: state.container_stats.openbmp_wrapped_payloads,
            unrecognized_payloads: state.container_stats.unrecognized_payloads,
            openbmp_unwrap_errors: state.container_stats.openbmp_unwrap_errors,
            inner_bmp_parse_errors: state.container_stats.inner_bmp_parse_errors,
        })
    } else {
        None
    };

    let summary = InspectSummary {
        file: &state.file_path,
        format: &state.format,
        size_bytes: state.file_size,
        total_messages: state.total_messages,
        malformed_messages: state.malformed_messages,
        bgp_elem_count: bgp_elems,
        by_type,
        peers_observed: state.peers.len(),
        active_peers,
        info_count,
        warn_count,
        error_count,
        findings_truncated: truncated,
        findings_dropped_count: state.findings_dropped,
        container,
    };

    let json = serde_json::to_string_pretty(&summary)
        .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string());
    println!("{json}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obmp_reader::ContainerStats;
    use crate::state::DoctorState;

    fn make_state(with_container: bool) -> DoctorState {
        let mut state = DoctorState::default();
        state.file_path = "test.rawbmp".into();
        state.format = "raw BMP frames".into();
        state.file_size = 100;
        state.total_messages = 3;
        if with_container {
            state.container_stats = ContainerStats {
                container_records: 3,
                openbmp_wrapped_payloads: 3,
                ..Default::default()
            };
        }
        state
    }

    #[test]
    fn test_summary_json_container_absent_for_raw_bmp() {
        let state = make_state(false);
        let summary = serde_json::to_string(&InspectSummary {
            file: &state.file_path,
            format: &state.format,
            size_bytes: state.file_size,
            total_messages: state.total_messages,
            malformed_messages: 0,
            bgp_elem_count: None,
            by_type: std::collections::BTreeMap::new(),
            peers_observed: 0,
            active_peers: 0,
            info_count: 0,
            warn_count: 0,
            error_count: 0,
            findings_truncated: false,
            findings_dropped_count: 0,
            container: None,
        })
        .unwrap();

        assert!(!summary.contains("container"));
    }

    #[test]
    fn test_summary_json_container_present_for_obmp() {
        let mut state = make_state(false);
        state.format = "OpenBMP length-delimited".into();
        state.container_stats = ContainerStats {
            container_records: 3,
            openbmp_wrapped_payloads: 3,
            ..Default::default()
        };

        let container = ContainerSummary {
            container_records: 3,
            raw_bmp_payloads: 0,
            openbmp_wrapped_payloads: 3,
            unrecognized_payloads: 0,
            openbmp_unwrap_errors: 0,
            inner_bmp_parse_errors: 0,
        };

        let summary = serde_json::to_string(&InspectSummary {
            file: &state.file_path,
            format: &state.format,
            size_bytes: state.file_size,
            total_messages: state.total_messages,
            malformed_messages: 0,
            bgp_elem_count: None,
            by_type: std::collections::BTreeMap::new(),
            peers_observed: 0,
            active_peers: 0,
            info_count: 0,
            warn_count: 0,
            error_count: 0,
            findings_truncated: false,
            findings_dropped_count: 0,
            container: Some(container),
        })
        .unwrap();

        assert!(summary.contains("container"));
        assert!(summary.contains("container_records"));
        assert!(summary.contains("openbmp_wrapped_payloads"));
        // Zero-value fields skipped via serde is_zero:
        assert!(!summary.contains("unrecognized_payloads"));
        assert!(!summary.contains("openbmp_unwrap_errors"));
        assert!(!summary.contains("inner_bmp_parse_errors"));
        assert!(!summary.contains("raw_bmp_payloads"));
    }
}
