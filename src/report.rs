use crate::raw_bmp::BmpMessageType;
use crate::state::{DoctorState, Finding, Severity};

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

pub fn render_inspect(state: &DoctorState) {
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

pub fn render_lint(findings: &[Finding]) {
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
}
