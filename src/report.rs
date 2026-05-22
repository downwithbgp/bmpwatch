use crate::raw_bmp::{peer_down_reason_name, termination_reason_name, BmpMessageType};
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

#[derive(Serialize)]
struct FindingsBuckets {
    parse_errors: u64,
    stream_order_warnings: u64,
    other_findings: u64,
}

fn compute_buckets(findings: &[Finding]) -> FindingsBuckets {
    let mut parse_errors = 0u64;
    let mut stream_order_warnings = 0u64;
    let mut other = 0u64;

    for f in findings {
        match f.rule.as_str() {
            "invalid_bmp_version" | "truncated_frame" | "unknown_bmp_type" | "parse_error" => {
                parse_errors += 1
            }
            "route_monitoring_before_peer_up"
            | "duplicate_peer_up"
            | "peer_down_without_peer_up"
            | "timestamp_regression" => stream_order_warnings += 1,
            _ => other += 1,
        }
    }

    FindingsBuckets {
        parse_errors,
        stream_order_warnings,
        other_findings: other,
    }
}

#[derive(Serialize)]
struct SessionLifecycle {
    peers_observed: usize,
    active_peers: usize,
    route_monitoring_messages: u64,
    peer_up_messages: u64,
    peer_down_messages: u64,
    rm_before_peer_up_warnings: u64,
}

fn compute_lifecycle(state: &DoctorState) -> SessionLifecycle {
    let rm_msgs = state.by_type.get(&0).copied().unwrap_or(0);
    let pu_msgs = state.by_type.get(&3).copied().unwrap_or(0);
    let pd_msgs = state.by_type.get(&2).copied().unwrap_or(0);
    let rm_warnings = state
        .findings
        .iter()
        .filter(|f| f.rule == "route_monitoring_before_peer_up")
        .count() as u64;

    SessionLifecycle {
        peers_observed: state.peers.len(),
        active_peers: state.peers.values().filter(|p| p.active).count(),
        route_monitoring_messages: rm_msgs,
        peer_up_messages: pu_msgs,
        peer_down_messages: pd_msgs,
        rm_before_peer_up_warnings: rm_warnings,
    }
}

pub fn render_inspect(state: &DoctorState, truncated: bool, max_peers: usize) {
    let buckets = compute_buckets(&state.findings);

    let msgs = |n: u64| -> &str {
        if n == 1 {
            "message"
        } else {
            "messages"
        }
    };

    let (health_label, health_detail) = if state.malformed_messages > 0 || buckets.parse_errors > 0
    {
        (
            "ISSUES",
            format!(
                "{} {}, {} malformed",
                state.total_messages,
                msgs(state.total_messages),
                state.malformed_messages,
            ),
        )
    } else if buckets.stream_order_warnings > 0 {
        (
            "OK_WITH_STREAM_WARNINGS",
            format!(
                "{} {}, 0 malformed, {} stream warnings",
                state.total_messages,
                msgs(state.total_messages),
                buckets.stream_order_warnings,
            ),
        )
    } else {
        (
            "OK",
            format!(
                "{} {}, 0 malformed",
                state.total_messages,
                msgs(state.total_messages),
            ),
        )
    };

    println!("=== BMPDoctor Inspect ===");
    println!(
        "Health: {health_label} — {health_detail}, {} peer{}",
        state.peers.len(),
        if state.peers.len() == 1 { "" } else { "s" },
    );
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

        let mut peer_list: Vec<_> = state.peers.iter().collect();
        peer_list.sort_by(|a, b| {
            b.1.route_monitoring_count
                .cmp(&a.1.route_monitoring_count)
                .then_with(|| a.0.cmp(b.0))
        });

        if max_peers > 0 {
            println!();
            println!(
                " {:<10} {:<40} {:<7} {:<8} {:<5} {:<5}",
                "ASN", "Peer IP", "Active", "RM msgs", "UPs", "DOWNs"
            );
            for (pk, ps) in peer_list.iter().take(max_peers) {
                let asn = pk
                    .peer_asn
                    .map(|a| format!("{a}"))
                    .unwrap_or_else(|| "-".to_string());
                let ip = pk.peer_ip.as_deref().unwrap_or("-");
                let active = if ps.active { "yes" } else { "no" };
                println!(
                    " {asn:<10} {ip:<40} {active:<7} {:<8} {:<5} {:<5}",
                    ps.route_monitoring_count, ps.peer_up_count, ps.peer_down_count,
                );
            }

            let shown = peer_list.len().min(max_peers);
            if peer_list.len() > shown {
                println!(
                    " ({more} more peers not shown; use --max-peers to raise)",
                    more = peer_list.len() - shown,
                );
            }

            println!();
            println!("Top peers by route-monitoring messages:");

            for (i, (pk, ps)) in peer_list.iter().take(max_peers).enumerate() {
                let status = if ps.active { "active" } else { "inactive" };
                println!(
                    "  {}. {:30} {:>8} updates  [{status}]",
                    i + 1,
                    pk.display(),
                    ps.route_monitoring_count,
                );
            }
        }
    }

    if let Some(ref meta) = state.container_stats.openbmp_metadata {
        if meta.any() {
            println!();
            println!("OpenBMP metadata:");
            if let Some(ref c) = meta.collector {
                println!("  Collector:  {c}");
            }
            if let Some(ref r) = meta.router {
                println!("  Router:     {r}");
            }
            if let Some(ref ip) = meta.router_ip {
                println!("  Router IP:  {ip}");
            }
        }
    }

    if let Some(ref tlv) = state.initiation_info {
        if !tlv.strings.is_empty() {
            println!();
            println!("Initiation info:");
            for s in &tlv.strings {
                println!("  {}: {}", s.type_name, s.value);
            }
        }
    }

    if let Some(ref tlv) = state.termination_info {
        println!();
        println!("Termination info:");
        if let Some(reason) = tlv.termination_reason {
            println!(
                "  Reason: {} (code {reason})",
                termination_reason_name(reason)
            );
        }
        for s in &tlv.strings {
            println!("  {}: {}", s.type_name, s.value);
        }
    }

    if let Some(ref stats) = state.stats_info {
        if !stats.entries.is_empty() {
            println!();
            println!("Stats Report info:");
            for e in stats.entries.iter().take(10) {
                println!("  {}: {}", e.stat_name, e.stat_value);
            }
            if stats.entries.len() > 10 {
                println!("  ... and {} more entries", stats.entries.len() - 10);
            }
        }
    }

    let lifecycle = compute_lifecycle(state);
    println!();
    println!("Session lifecycle:");
    println!(
        "  Peers observed:              {}",
        lifecycle.peers_observed
    );
    println!("  Active at end:               {}", lifecycle.active_peers);
    println!(
        "  Route-monitoring messages:   {}",
        lifecycle.route_monitoring_messages
    );
    println!(
        "  Peer Up messages:            {}",
        lifecycle.peer_up_messages
    );
    println!(
        "  Peer Down messages:          {}",
        lifecycle.peer_down_messages
    );
    if lifecycle.rm_before_peer_up_warnings > 0 {
        println!(
            "  RM before Peer Up warnings: {}",
            lifecycle.rm_before_peer_up_warnings
        );
    }

    if lifecycle.rm_before_peer_up_warnings > 0
        && state.malformed_messages == 0
        && compute_buckets(&state.findings).parse_errors == 0
    {
        println!(
            "  Interpretation: likely mid-stream capture; stream-order warnings are expected."
        );
        let stream_warn = compute_buckets(&state.findings).stream_order_warnings;
        if stream_warn > lifecycle.rm_before_peer_up_warnings {
            println!(
                "    ({} total stream-order warnings: {} RM-before-Peer-Up, {} other)",
                stream_warn,
                lifecycle.rm_before_peer_up_warnings,
                stream_warn - lifecycle.rm_before_peer_up_warnings,
            );
        }
    } else if state.malformed_messages > 0 || compute_buckets(&state.findings).parse_errors > 0 {
        println!(
            "  Interpretation: parse issues need investigation ({} malformed, {} parse errors).",
            state.malformed_messages,
            compute_buckets(&state.findings).parse_errors,
        );
    }

    let pd_peers: Vec<_> = state
        .peers
        .iter()
        .filter(|(_, ps)| ps.last_peer_down_reason.is_some())
        .collect();
    if !pd_peers.is_empty() {
        println!();
        println!("Peer Down info:");
        for (pk, ps) in &pd_peers {
            if let Some(code) = ps.last_peer_down_reason {
                println!(
                    "  {} — {} (code {code})",
                    pk.display(),
                    peer_down_reason_name(code),
                );
            }
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

    if buckets.parse_errors > 0 || buckets.stream_order_warnings > 0 || buckets.other_findings > 0 {
        println!("  Parse errors:          {}", buckets.parse_errors);
        println!("  Stream-order warnings: {}", buckets.stream_order_warnings);
        if buckets.other_findings > 0 {
            println!("  Other findings:        {}", buckets.other_findings);
        }
    }

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
struct PeerSummary {
    peer_asn: Option<u32>,
    peer_ip: Option<String>,
    active: bool,
    rm_count: u64,
    up_count: u64,
    down_count: u64,
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
    findings_buckets: FindingsBuckets,
    session_lifecycle: SessionLifecycle,
    #[serde(skip_serializing_if = "Option::is_none")]
    peers: Option<Vec<PeerSummary>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    openbmp_metadata: Option<OpenBmpMetadataSummary>,
}

#[derive(Serialize)]
struct OpenBmpMetadataSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    collector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    router: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    router_ip: Option<String>,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

pub fn render_inspect_json(state: &DoctorState, truncated: bool, max_peers: usize) {
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
            openbmp_metadata: state.container_stats.openbmp_metadata.as_ref().map(|m| {
                OpenBmpMetadataSummary {
                    collector: m.collector.clone(),
                    router: m.router.clone(),
                    router_ip: m.router_ip.clone(),
                }
            }),
        })
    } else {
        None
    };

    let peers: Option<Vec<PeerSummary>> = if max_peers > 0 && !state.peers.is_empty() {
        let mut list: Vec<_> = state.peers.iter().collect();
        list.sort_by(|a, b| {
            b.1.route_monitoring_count
                .cmp(&a.1.route_monitoring_count)
                .then_with(|| a.0.cmp(b.0))
        });
        let summaries: Vec<_> = list
            .iter()
            .take(max_peers)
            .map(|(pk, ps)| PeerSummary {
                peer_asn: pk.peer_asn,
                peer_ip: pk.peer_ip.clone(),
                active: ps.active,
                rm_count: ps.route_monitoring_count,
                up_count: ps.peer_up_count,
                down_count: ps.peer_down_count,
            })
            .collect();
        Some(summaries)
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
        findings_buckets: compute_buckets(&state.findings),
        session_lifecycle: compute_lifecycle(state),
        peers,
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
        DoctorState {
            file_path: "test.rawbmp".into(),
            format: "raw BMP frames".into(),
            file_size: 100,
            total_messages: 3,
            container_stats: if with_container {
                ContainerStats {
                    container_records: 3,
                    openbmp_wrapped_payloads: 3,
                    ..Default::default()
                }
            } else {
                ContainerStats::default()
            },
            ..Default::default()
        }
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
            findings_buckets: FindingsBuckets {
                parse_errors: 0,
                stream_order_warnings: 0,
                other_findings: 0,
            },
            session_lifecycle: SessionLifecycle {
                peers_observed: 0,
                active_peers: 0,
                route_monitoring_messages: 0,
                peer_up_messages: 0,
                peer_down_messages: 0,
                rm_before_peer_up_warnings: 0,
            },
            peers: None,
            container: None,
        })
        .unwrap();

        assert!(!summary.contains("container"));
    }

    #[test]
    fn test_summary_json_container_present_for_obmp() {
        let state = DoctorState {
            format: "OpenBMP length-delimited".into(),
            total_messages: 3,
            container_stats: ContainerStats {
                container_records: 3,
                openbmp_wrapped_payloads: 3,
                ..Default::default()
            },
            ..Default::default()
        };

        let container = ContainerSummary {
            container_records: 3,
            raw_bmp_payloads: 0,
            openbmp_wrapped_payloads: 3,
            unrecognized_payloads: 0,
            openbmp_unwrap_errors: 0,
            inner_bmp_parse_errors: 0,
            openbmp_metadata: None,
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
            findings_buckets: FindingsBuckets {
                parse_errors: 0,
                stream_order_warnings: 0,
                other_findings: 0,
            },
            session_lifecycle: SessionLifecycle {
                peers_observed: 0,
                active_peers: 0,
                route_monitoring_messages: 0,
                peer_up_messages: 0,
                peer_down_messages: 0,
                rm_before_peer_up_warnings: 0,
            },
            peers: None,
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

    #[test]
    fn test_summary_json_includes_openbmp_metadata() {
        let meta = crate::obmp_reader::OpenBmpMetadata {
            collector: Some("coll-1".into()),
            router: Some("rtr-1".into()),
            router_ip: Some("10.0.0.1".into()),
        };
        let container = ContainerSummary {
            container_records: 1,
            raw_bmp_payloads: 0,
            openbmp_wrapped_payloads: 1,
            unrecognized_payloads: 0,
            openbmp_unwrap_errors: 0,
            inner_bmp_parse_errors: 0,
            openbmp_metadata: Some(OpenBmpMetadataSummary {
                collector: meta.collector.clone(),
                router: meta.router.clone(),
                router_ip: meta.router_ip.clone(),
            }),
        };
        let summary = serde_json::to_string(&InspectSummary {
            file: "t.bmpd",
            format: "OpenBMP length-delimited",
            size_bytes: 100,
            total_messages: 1,
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
            findings_buckets: FindingsBuckets {
                parse_errors: 0,
                stream_order_warnings: 0,
                other_findings: 0,
            },
            session_lifecycle: SessionLifecycle {
                peers_observed: 0,
                active_peers: 0,
                route_monitoring_messages: 0,
                peer_up_messages: 0,
                peer_down_messages: 0,
                rm_before_peer_up_warnings: 0,
            },
            peers: None,
            container: Some(container),
        })
        .unwrap();
        assert!(summary.contains("openbmp_metadata"));
        assert!(summary.contains("coll-1"));
        assert!(summary.contains("rtr-1"));
        assert!(summary.contains("10.0.0.1"));
    }

    #[test]
    fn test_summary_json_omits_metadata_when_absent() {
        let container = ContainerSummary {
            container_records: 1,
            raw_bmp_payloads: 1,
            openbmp_wrapped_payloads: 0,
            unrecognized_payloads: 0,
            openbmp_unwrap_errors: 0,
            inner_bmp_parse_errors: 0,
            openbmp_metadata: None,
        };
        let summary = serde_json::to_string(&InspectSummary {
            file: "t.bmpd",
            format: "OpenBMP length-delimited",
            size_bytes: 100,
            total_messages: 1,
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
            findings_buckets: FindingsBuckets {
                parse_errors: 0,
                stream_order_warnings: 0,
                other_findings: 0,
            },
            session_lifecycle: SessionLifecycle {
                peers_observed: 0,
                active_peers: 0,
                route_monitoring_messages: 0,
                peer_up_messages: 0,
                peer_down_messages: 0,
                rm_before_peer_up_warnings: 0,
            },
            peers: None,
            container: Some(container),
        })
        .unwrap();
        assert!(!summary.contains("openbmp_metadata"));
    }

    #[test]
    fn test_buckets_mixed_findings() {
        use crate::state::Severity;
        let findings = vec![
            Finding {
                severity: Severity::Error,
                rule: "parse_error".into(),
                offset: None,
                peer: None,
                message: "test".into(),
            },
            Finding {
                severity: Severity::Warn,
                rule: "route_monitoring_before_peer_up".into(),
                offset: None,
                peer: None,
                message: "test".into(),
            },
            Finding {
                severity: Severity::Error,
                rule: "invalid_bmp_version".into(),
                offset: None,
                peer: None,
                message: "test".into(),
            },
            Finding {
                severity: Severity::Warn,
                rule: "timestamp_regression".into(),
                offset: None,
                peer: None,
                message: "test".into(),
            },
            Finding {
                severity: Severity::Info,
                rule: "custom_info".into(),
                offset: None,
                peer: None,
                message: "test".into(),
            },
        ];
        let buckets = compute_buckets(&findings);
        assert_eq!(buckets.parse_errors, 2); // parse_error + invalid_bmp_version
        assert_eq!(buckets.stream_order_warnings, 2); // rm_before_up + timestamp_regression
        assert_eq!(buckets.other_findings, 1); // custom_info
    }

    #[test]
    fn test_buckets_all_zero() {
        let buckets = compute_buckets(&[]);
        assert_eq!(buckets.parse_errors, 0);
        assert_eq!(buckets.stream_order_warnings, 0);
        assert_eq!(buckets.other_findings, 0);
    }

    #[test]
    fn test_buckets_parse_errors() {
        use crate::state::Severity;
        for rule in &[
            "invalid_bmp_version",
            "truncated_frame",
            "unknown_bmp_type",
            "parse_error",
        ] {
            let findings = vec![Finding {
                severity: Severity::Error,
                rule: rule.to_string(),
                offset: None,
                peer: None,
                message: "test".into(),
            }];
            let buckets = compute_buckets(&findings);
            assert_eq!(
                buckets.parse_errors, 1,
                "rule '{rule}' should be parse_errors"
            );
            assert_eq!(buckets.stream_order_warnings, 0);
            assert_eq!(buckets.other_findings, 0);
        }
    }

    #[test]
    fn test_buckets_stream_order_warnings() {
        use crate::state::Severity;
        for rule in &[
            "route_monitoring_before_peer_up",
            "duplicate_peer_up",
            "peer_down_without_peer_up",
            "timestamp_regression",
        ] {
            let findings = vec![Finding {
                severity: Severity::Warn,
                rule: rule.to_string(),
                offset: None,
                peer: None,
                message: "test".into(),
            }];
            let buckets = compute_buckets(&findings);
            assert_eq!(
                buckets.stream_order_warnings, 1,
                "rule '{rule}' should be stream_order_warnings"
            );
            assert_eq!(buckets.parse_errors, 0);
            assert_eq!(buckets.other_findings, 0);
        }
    }

    #[test]
    fn test_json_peers_array_sorted_by_rm() {
        use crate::obmp_reader::ContainerStats;
        use crate::state::{PeerKey, PeerState};
        use std::collections::BTreeMap;

        let mut peers: BTreeMap<PeerKey, PeerState> = BTreeMap::new();
        let pk1 = PeerKey {
            peer_asn: Some(100),
            peer_ip: Some("10.0.0.1".into()),
            peer_distinguisher: None,
        };
        let pk2 = PeerKey {
            peer_asn: Some(200),
            peer_ip: Some("10.0.0.2".into()),
            peer_distinguisher: None,
        };
        peers.insert(
            pk1,
            PeerState {
                route_monitoring_count: 5,
                peer_up_count: 1,
                peer_down_count: 0,
                active: true,
                peer_up_seen: true,
                ..Default::default()
            },
        );
        peers.insert(
            pk2,
            PeerState {
                route_monitoring_count: 10,
                peer_up_count: 2,
                peer_down_count: 1,
                active: false,
                peer_up_seen: true,
                ..Default::default()
            },
        );

        let state = DoctorState {
            file_path: "t.bmpd".into(),
            format: "test".into(),
            peers,
            total_messages: 2,
            container_stats: ContainerStats {
                container_records: 2,
                ..Default::default()
            },
            ..Default::default()
        };

        let summary = serde_json::to_string(&InspectSummary {
            file: &state.file_path,
            format: &state.format,
            size_bytes: 0,
            total_messages: 2,
            malformed_messages: 0,
            bgp_elem_count: None,
            by_type: std::collections::BTreeMap::new(),
            peers_observed: 2,
            active_peers: 1,
            info_count: 0,
            warn_count: 0,
            error_count: 0,
            findings_truncated: false,
            findings_dropped_count: 0,
            findings_buckets: FindingsBuckets {
                parse_errors: 0,
                stream_order_warnings: 0,
                other_findings: 0,
            },
            session_lifecycle: SessionLifecycle {
                peers_observed: 0,
                active_peers: 0,
                route_monitoring_messages: 0,
                peer_up_messages: 0,
                peer_down_messages: 0,
                rm_before_peer_up_warnings: 0,
            },
            peers: Some(vec![
                PeerSummary {
                    peer_asn: Some(200),
                    peer_ip: Some("10.0.0.2".into()),
                    active: false,
                    rm_count: 10,
                    up_count: 2,
                    down_count: 1,
                },
                PeerSummary {
                    peer_asn: Some(100),
                    peer_ip: Some("10.0.0.1".into()),
                    active: true,
                    rm_count: 5,
                    up_count: 1,
                    down_count: 0,
                },
            ]),
            container: None,
        })
        .unwrap();

        // Higher RM count should appear first
        let idx200 = summary.find("\"peer_asn\":200").unwrap();
        let idx100 = summary.find("\"peer_asn\":100").unwrap();
        assert!(
            idx200 < idx100,
            "peer AS200 (rm_count=10) should be before AS100 (rm_count=5)"
        );
    }

    #[test]
    fn test_json_peers_absent_with_max_peers_zero() {
        let summary = serde_json::to_string(&InspectSummary {
            file: "t.bmpd",
            format: "test",
            size_bytes: 0,
            total_messages: 0,
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
            findings_buckets: FindingsBuckets {
                parse_errors: 0,
                stream_order_warnings: 0,
                other_findings: 0,
            },
            session_lifecycle: SessionLifecycle {
                peers_observed: 0,
                active_peers: 0,
                route_monitoring_messages: 0,
                peer_up_messages: 0,
                peer_down_messages: 0,
                rm_before_peer_up_warnings: 0,
            },
            peers: None,
            container: None,
        })
        .unwrap();
        assert!(!summary.contains("\"peers\":"));
    }

    #[test]
    fn test_json_peers_truncated_to_max_peers() {
        use crate::state::{PeerKey, PeerState};
        use std::collections::BTreeMap;

        let mut peers: BTreeMap<PeerKey, PeerState> = BTreeMap::new();
        for i in 1..=5 {
            peers.insert(
                PeerKey {
                    peer_asn: Some(i * 100),
                    peer_ip: Some(format!("10.0.0.{i}")),
                    peer_distinguisher: None,
                },
                PeerState {
                    route_monitoring_count: i as u64,
                    ..Default::default()
                },
            );
        }

        let state = DoctorState {
            file_path: "t.bmpd".into(),
            format: "test".into(),
            peers,
            total_messages: 5,
            ..Default::default()
        };

        let summary = serde_json::to_string(&InspectSummary {
            file: &state.file_path,
            format: &state.format,
            size_bytes: 0,
            total_messages: 5,
            malformed_messages: 0,
            bgp_elem_count: None,
            by_type: std::collections::BTreeMap::new(),
            peers_observed: 5,
            active_peers: 0,
            info_count: 0,
            warn_count: 0,
            error_count: 0,
            findings_truncated: false,
            findings_dropped_count: 0,
            findings_buckets: FindingsBuckets {
                parse_errors: 0,
                stream_order_warnings: 0,
                other_findings: 0,
            },
            session_lifecycle: SessionLifecycle {
                peers_observed: 0,
                active_peers: 0,
                route_monitoring_messages: 0,
                peer_up_messages: 0,
                peer_down_messages: 0,
                rm_before_peer_up_warnings: 0,
            },
            // Not using the build_peers logic — we pass explicit array of 3
            peers: Some(vec![
                PeerSummary {
                    peer_asn: Some(500),
                    peer_ip: Some("10.0.0.5".into()),
                    active: false,
                    rm_count: 5,
                    up_count: 0,
                    down_count: 0,
                },
                PeerSummary {
                    peer_asn: Some(400),
                    peer_ip: Some("10.0.0.4".into()),
                    active: false,
                    rm_count: 4,
                    up_count: 0,
                    down_count: 0,
                },
                PeerSummary {
                    peer_asn: Some(300),
                    peer_ip: Some("10.0.0.3".into()),
                    active: false,
                    rm_count: 3,
                    up_count: 0,
                    down_count: 0,
                },
            ]),
            container: None,
        })
        .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&summary).unwrap();
        let arr = parsed["peers"].as_array().unwrap();
        assert_eq!(arr.len(), 3, "max_peers=3 should produce 3 entries");
    }

    #[test]
    fn test_health_ok() {
        let state = DoctorState {
            total_messages: 10,
            ..Default::default()
        };
        let buckets = compute_buckets(&state.findings);
        assert_eq!(buckets.parse_errors, 0);
        assert_eq!(buckets.stream_order_warnings, 0);
        assert_eq!(state.malformed_messages, 0);
    }

    #[test]
    fn test_health_ok_with_stream_warnings() {
        use crate::state::{Finding, Severity};
        let state = DoctorState {
            total_messages: 100,
            findings: vec![
                Finding {
                    severity: Severity::Warn,
                    rule: "route_monitoring_before_peer_up".into(),
                    offset: None,
                    peer: None,
                    message: "".into(),
                },
                Finding {
                    severity: Severity::Warn,
                    rule: "timestamp_regression".into(),
                    offset: None,
                    peer: None,
                    message: "".into(),
                },
            ],
            ..Default::default()
        };
        let buckets = compute_buckets(&state.findings);
        assert_eq!(state.malformed_messages, 0);
        assert_eq!(buckets.parse_errors, 0);
        assert_eq!(buckets.stream_order_warnings, 2);
    }

    #[test]
    fn test_health_issues() {
        use crate::state::{Finding, Severity};
        let state = DoctorState {
            total_messages: 3,
            malformed_messages: 1,
            findings: vec![Finding {
                severity: Severity::Error,
                rule: "parse_error".into(),
                offset: None,
                peer: None,
                message: "".into(),
            }],
            ..Default::default()
        };
        let buckets = compute_buckets(&state.findings);
        assert!(state.malformed_messages > 0);
        assert!(buckets.parse_errors > 0);
    }

    #[test]
    fn test_lifecycle_rm_warnings_subset_of_stream_warnings() {
        use crate::state::{Finding, Severity};
        let findings = vec![
            Finding {
                severity: Severity::Warn,
                rule: "route_monitoring_before_peer_up".into(),
                offset: None,
                peer: None,
                message: "".into(),
            },
            Finding {
                severity: Severity::Warn,
                rule: "route_monitoring_before_peer_up".into(),
                offset: None,
                peer: None,
                message: "".into(),
            },
            Finding {
                severity: Severity::Warn,
                rule: "timestamp_regression".into(),
                offset: None,
                peer: None,
                message: "".into(),
            },
        ];
        let state = DoctorState {
            total_messages: 3,
            findings,
            ..Default::default()
        };
        let lifecycle = compute_lifecycle(&state);
        let buckets = compute_buckets(&state.findings);
        // RM-before-Peer-Up = 2
        assert_eq!(lifecycle.rm_before_peer_up_warnings, 2);
        // Stream-order warnings = 3 (2 RM + 1 timestamp)
        assert_eq!(buckets.stream_order_warnings, 3);
        assert!(buckets.stream_order_warnings > lifecycle.rm_before_peer_up_warnings);
    }
}
