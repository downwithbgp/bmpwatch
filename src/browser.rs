use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Stylize;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::DefaultTerminal;

use crate::dashboard::as_name;

pub(crate) struct ParsedTopic {
    pub(crate) collector: String,
    pub(crate) asn_str: String,
    pub(crate) full: String,
}

pub(crate) fn parse_topic(t: &str) -> Option<ParsedTopic> {
    let body = t.strip_prefix("routeviews.")?;
    let body = body.strip_suffix(".bmp_raw")?;
    let mut parts: Vec<&str> = body.splitn(2, '.').collect();
    if parts.len() < 2 {
        return Some(ParsedTopic {
            collector: parts[0].to_string(),
            asn_str: "-".to_string(),
            full: t.to_string(),
        });
    }
    let asn_str = parts.pop().unwrap().to_string();
    let collector = parts.join(".");
    if collector.is_empty() {
        return None;
    }
    Some(ParsedTopic {
        collector,
        asn_str,
        full: t.to_string(),
    })
}

pub(crate) fn collector_region(collector: &str) -> &str {
    let c = collector.to_lowercase();
    match c.as_str() {
        "chicago" | "nwax" | "ny" | "pit" | "sfmix" | "phoix" | "telxatl"
        | "interlan" | "isc" | "pacwave" | "fortaleza" | "pitmx"
        | "route-views" | "route-views2" | "route-views4" | "route-views6" => "North America",
        "linx" | "amsix" | "decix" | "flix" | "netnod" | "spb" | "namex"
        | "cix" | "siex" | "gorex" | "frr" | "wide"
        | "route-views3" | "route-views5" => "Europe",
        "sg" | "hkix" | "kixp" | "kinx" | "bknix" | "iix" | "eqix"
        | "mwix" | "bdix" | "getafix" | "sydney" | "perth"
        | "route-views7" | "route-views8" => "Asia Pacific",
        "chile" | "crix" | "ix-br" | "ix-br2" | "rio" | "saopaulo2"
        | "soxrs" | "peru" | "locix" | "ixpn" => "Latin America",
        "jhb" | "napafrica" | "iraq-ixp" | "gixa" | "uaeix" => "Africa / Middle East",
        _ => "Other",
    }
}

struct RegionInfo {
    name: &'static str,
    stream_count: usize,
}

pub(crate) fn topic_browser(
    terminal: &mut DefaultTerminal,
    topics: &[String],
) -> Result<Option<String>> {
    let parsed: Vec<ParsedTopic> = topics.iter().filter_map(|t| parse_topic(t)).collect();

    let mut region_map: BTreeMap<&str, Vec<&ParsedTopic>> = BTreeMap::new();
    for pt in &parsed {
        region_map
            .entry(collector_region(&pt.collector))
            .or_default()
            .push(pt);
    }

    let region_order: [&str; 6] = [
        "North America", "Europe", "Asia Pacific",
        "Latin America", "Africa / Middle East", "Other",
    ];
    let regions: Vec<RegionInfo> = region_order
        .iter()
        .filter_map(|name| region_map.get(name).map(|v| RegionInfo {
            name: *name,
            stream_count: v.len(),
        }))
        .collect();

    // ---------- Screen 1: Regions ----------
    let mut filter = String::new();
    let mut selected: usize = 0;

    let chosen_region: &str = loop {
        let filtered_regions: Vec<&RegionInfo> = if filter.is_empty() {
            regions.iter().collect()
        } else {
            let lower = filter.to_lowercase();
            regions.iter().filter(|r| r.name.to_lowercase().contains(&lower)).collect()
        };

        if !filtered_regions.is_empty() && selected >= filtered_regions.len() {
            selected = filtered_regions.len() - 1;
        }

        terminal.draw(|f| {
            let chunks = Layout::vertical([
                Constraint::Length(1), Constraint::Length(1),
                Constraint::Min(0), Constraint::Length(1),
            ]).split(f.area());

            f.render_widget(
                Paragraph::new(" BMPDoctor — Regions ").bold()
                    .block(Block::bordered().borders(Borders::ALL)).centered(),
                chunks[0],
            );
            f.render_widget(
                Paragraph::new(format!(" Filter: {filter}"))
                    .block(Block::bordered().borders(Borders::ALL)),
                chunks[1],
            );

            let mut lines: Vec<Line> = Vec::new();
            for (i, r) in filtered_regions.iter().enumerate() {
                let text = format!(" {}{:<25} {:>5} streams",
                    if i == selected { ">" } else { " " }, r.name, r.stream_count);
                lines.push(if i == selected {
                    Line::from(text).on_white().black()
                } else {
                    Line::from(text)
                });
            }
            if lines.is_empty() {
                lines.push(Line::from(" (no matches)").dark_gray());
            }
            f.render_widget(
                Paragraph::new(Text::from(lines)).block(Block::bordered().borders(Borders::ALL)),
                chunks[2],
            );
            f.render_widget(
                Paragraph::new(" [↑↓] navigate  [type] filter  [enter] select  [esc] quit ")
                    .block(Block::bordered().borders(Borders::ALL)).centered(),
                chunks[3],
            );
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press { continue; }
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                    KeyCode::Enter => {
                        if let Some(r) = filtered_regions.get(selected) { break r.name; }
                    }
                    KeyCode::Char(c) => { filter.push(c); selected = 0; }
                    KeyCode::Backspace => { filter.pop(); selected = 0; }
                    KeyCode::Up => { if selected > 0 { selected -= 1; } }
                    KeyCode::Down => { if selected + 1 < filtered_regions.len() { selected += 1; } }
                    _ => {}
                }
            }
        }
    };

    // ---------- Screen 2: Collectors ----------
    let region_topics: Vec<&ParsedTopic> = region_map
        .get(chosen_region).map(|v| v.as_slice()).unwrap_or(&[]).to_vec();

    let mut collector_groups: BTreeMap<&str, Vec<&ParsedTopic>> = BTreeMap::new();
    for pt in &region_topics {
        collector_groups.entry(pt.collector.as_str()).or_default().push(pt);
    }
    let mut collectors: Vec<(&str, usize)> = collector_groups
        .iter().map(|(name, streams)| (*name, streams.len())).collect();
    collectors.sort_by(|a, b| b.1.cmp(&a.1));

    filter.clear();
    selected = 0;

    let chosen_collector: String = loop {
        let filtered: Vec<&(&str, usize)> = if filter.is_empty() {
            collectors.iter().collect()
        } else {
            let lower = filter.to_lowercase();
            collectors.iter().filter(|(name, _)| name.to_lowercase().contains(&lower)).collect()
        };
        if !filtered.is_empty() && selected >= filtered.len() {
            selected = filtered.len() - 1;
        }

        terminal.draw(|f| {
            let chunks = Layout::vertical([
                Constraint::Length(1), Constraint::Length(1),
                Constraint::Min(0), Constraint::Length(1),
            ]).split(f.area());

            f.render_widget(
                Paragraph::new(format!(" BMPDoctor — {chosen_region} — Collectors ")).bold()
                    .block(Block::bordered().borders(Borders::ALL)).centered(),
                chunks[0],
            );
            f.render_widget(
                Paragraph::new(format!(" Filter: {filter}"))
                    .block(Block::bordered().borders(Borders::ALL)),
                chunks[1],
            );

            let mut lines: Vec<Line> = Vec::new();
            for (i, (name, count)) in filtered.iter().enumerate() {
                let text = format!(" {}{name:<30} {count:>5} streams",
                    if i == selected { ">" } else { " " });
                lines.push(if i == selected {
                    Line::from(text).on_white().black()
                } else {
                    Line::from(text)
                });
            }
            if lines.is_empty() {
                lines.push(Line::from(" (no matches)").dark_gray());
            }
            f.render_widget(
                Paragraph::new(Text::from(lines)).block(Block::bordered().borders(Borders::ALL)),
                chunks[2],
            );
            f.render_widget(
                Paragraph::new(" [↑↓] navigate  [type] filter  [enter] select  [esc] back ")
                    .block(Block::bordered().borders(Borders::ALL)).centered(),
                chunks[3],
            );
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press { continue; }
                match key.code {
                    KeyCode::Esc => return topic_browser(terminal, topics),
                    KeyCode::Char('q') => return Ok(None),
                    KeyCode::Enter => {
                        if let Some((name, _)) = filtered.get(selected) { break name.to_string(); }
                    }
                    KeyCode::Char(c) => { filter.push(c); selected = 0; }
                    KeyCode::Backspace => { filter.pop(); selected = 0; }
                    KeyCode::Up => { if selected > 0 { selected -= 1; } }
                    KeyCode::Down => { if selected + 1 < filtered.len() { selected += 1; } }
                    _ => {}
                }
            }
        }
    };

    // ---------- Screen 3: Streams ----------
    let collector_streams: Vec<&&ParsedTopic> = region_topics
        .iter().filter(|pt| pt.collector == chosen_collector).collect();

    filter.clear();
    selected = 0;

    loop {
        let filtered: Vec<&&&ParsedTopic> = if filter.is_empty() {
            collector_streams.iter().collect()
        } else {
            let lower = filter.to_lowercase();
            collector_streams.iter().filter(|pt| {
                pt.asn_str.contains(&lower) || pt.full.to_lowercase().contains(&lower)
            }).collect()
        };
        if !filtered.is_empty() && selected >= filtered.len() {
            selected = filtered.len() - 1;
        }

        terminal.draw(|f| {
            let chunks = Layout::vertical([
                Constraint::Length(1), Constraint::Length(1),
                Constraint::Min(0), Constraint::Length(1),
            ]).split(f.area());

            f.render_widget(
                Paragraph::new(format!(" BMPDoctor — {chosen_region} — {chosen_collector} ")).bold()
                    .block(Block::bordered().borders(Borders::ALL)).centered(),
                chunks[0],
            );
            f.render_widget(
                Paragraph::new(format!(" Filter: {filter}"))
                    .block(Block::bordered().borders(Borders::ALL)),
                chunks[1],
            );

            let mut lines: Vec<Line> = Vec::new();
            for (i, pt) in filtered.iter().enumerate() {
                let name = as_name(&pt.asn_str);
                let text = format!(" {}AS{:<8} {:<24} {}",
                    if i == selected { ">" } else { " " }, pt.asn_str, name, pt.full);
                lines.push(if i == selected {
                    Line::from(text).on_white().black()
                } else {
                    Line::from(text)
                });
            }
            if lines.is_empty() {
                lines.push(Line::from(" (no matches)").dark_gray());
            }
            f.render_widget(
                Paragraph::new(Text::from(lines)).block(Block::bordered().borders(Borders::ALL)),
                chunks[2],
            );
            f.render_widget(
                Paragraph::new(format!(
                    " [↑↓] navigate  [type] filter  [enter] connect  [esc] back  {} streams",
                    collector_streams.len()
                )).block(Block::bordered().borders(Borders::ALL)).centered(),
                chunks[3],
            );
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press { continue; }
                match key.code {
                    KeyCode::Esc => break,
                    KeyCode::Char('q') => return Ok(None),
                    KeyCode::Enter => {
                        if let Some(pt) = filtered.get(selected) {
                            return Ok(Some(pt.full.clone()));
                        }
                    }
                    KeyCode::Char(c) => { filter.push(c); selected = 0; }
                    KeyCode::Backspace => { filter.pop(); selected = 0; }
                    KeyCode::Up => { if selected > 0 { selected -= 1; } }
                    KeyCode::Down => { if selected + 1 < filtered.len() { selected += 1; } }
                    _ => {}
                }
            }
        }
    }

    topic_browser(terminal, topics)
}
