use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span, Text};
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
    let (collector, asn_str) = match body.rsplit_once('.') {
        Some((col, asn)) => (col.to_string(), asn.to_string()),
        None => {
            return Some(ParsedTopic {
                collector: body.to_string(),
                asn_str: "-".to_string(),
                full: t.to_string(),
            });
        }
    };
    if collector.is_empty() {
        return None;
    }
    Some(ParsedTopic {
        collector,
        asn_str,
        full: t.to_string(),
    })
}

// Persist recently connected streams so they appear at the top
fn recent_cache() -> &'static Mutex<Vec<String>> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

fn add_recent(topic: &str) {
    let mut recent = recent_cache().lock().unwrap();
    recent.retain(|t| t != topic);
    recent.insert(0, topic.to_string());
    recent.truncate(10);
}

// Collector display labels derived from the public RouteViews Looking
// Glass collector list (https://www.routeviews.org/lg/). Topic names
// remain authoritative; these labels are descriptive UI hints only.
// BMPWatch is not affiliated with RouteViews.
fn collector_label(name: &str) -> &str {
    match name {
        "amsix.ams" => "Amsterdam, Netherlands (AMS-IX)",
        "cix.atl" => "Atlanta, Georgia (CIX-ATL)",
        "crix.sjo" => "San José, Costa Rica (CRIX)",
        "decix.fra" => "Frankfurt, Germany (DE-CIX)",
        "decix.jhb" => "Johor Bahru, Malaysia (DE-CIX)",
        "getafix.mnl" => "Manila, Philippines (GetaFIX)",
        "hkix.hkg" => "Hong Kong (HKIX)",
        "iix.cgk" => "Jakarta, Indonesia (IIX)",
        "interlan.otp" => "Bucharest, Romania (InterLAN-IX)",
        "iraq-ixp.bgw" => "Baghdad, Iraq (IRAQ-IXP)",
        "ix-br.gru" => "São Paulo, Brazil (IX.br)",
        "ix-br2.gru" => "São Paulo, Brazil (IX.br 2)",
        "ixpn.los" => "Lagos, Nigeria (IXPN)",
        "kinx.icn" => "Seoul, Korea (KINX)",
        "locix.fra" => "Frankfurt, Germany (LocIX)",
        "namex.fco" => "Rome, Italy (NAMEX)",
        "netnod.mmx" => "Malmö, Sweden (Netnod)",
        "pacwave.lax" => "Los Angeles, California (Pacific Wave)",
        "pit.scl" => "Santiago, Chile (PIT Chile)",
        "pitmx.qro" => "Querétaro, Mexico (PIT Chile MX)",
        "route-views.bdix" => "Dhaka, Bangladesh (BDIX)",
        "route-views.bknix" => "Bangkok, Thailand (BKNIX)",
        "route-views.chicago" => "Chicago, Illinois (Equinix CH1)",
        "route-views.chile" => "Santiago, Chile (NIC.cl)",
        "route-views.eqix" => "Ashburn, Virginia (Equinix)",
        "route-views.flix" => "Miami, Florida (FL-IX)",
        "route-views.fortaleza" => "Fortaleza, Brazil (IX.br)",
        "route-views.gixa" => "Accra, Ghana (GIXA)",
        "route-views.gorex" => "Guam, US Territories (GOREX)",
        "route-views.isc" => "Palo Alto, California (PAIX)",
        "route-views.kixp" => "Nairobi, Kenya (KIXP)",
        "route-views.linx" => "London, United Kingdom (LINX)",
        "route-views.mwix" => "Indianapolis, Indiana (FD-IX)",
        "route-views.napafrica" => "Johannesburg, South Africa (NAPAfrica)",
        "route-views.nwax" => "Portland, Oregon (NWAX)",
        "route-views.ny" => "New York, NY (DE-CIX)",
        "route-views.perth" => "Perth, Australia (WA-IX)",
        "route-views.peru" => "Lima, Peru (Peru IX)",
        "route-views.phoix" => "Quezon City, Philippines (PhOpenIX)",
        "route-views.rio" => "Rio de Janeiro, Brazil (IX.br)",
        "route-views.sfmix" => "San Francisco, California (SFMIX)",
        "route-views.sg" => "Singapore (Equinix)",
        "route-views.soxrs" => "Belgrade, Serbia (SOX)",
        "route-views.sydney" => "Sydney, Australia (Equinix SYD1)",
        "route-views.telxatl" => "Atlanta, Georgia (Digital Realty)",
        "route-views.uaeix" => "Dubai, UAE (UAE-IX)",
        "route-views.wide" => "Tokyo, Japan (DIX-IE)",
        "route-views2" => "Multi-hop 2 (Univ of Oregon)",
        "route-views3" => "Multi-hop 3 (Univ of Oregon)",
        "route-views4" => "Multi-hop 4 (Univ of Oregon)",
        "route-views5" => "Multi-hop 5 (Univ of Oregon)",
        "route-views6" => "Multi-hop 6 (Univ of Oregon)",
        "route-views7" => "Multi-hop 7 (Univ of Oregon)",
        "route-views8" => "Multi-hop 8 (Univ of Oregon)",
        "frr" => "Test collector (FRR, Univ of Oregon)",
        _ => name,
    }
}

struct CollectorInfo<'a> {
    raw_name: String,
    label: String,
    stream_count: usize,
    streams: Vec<&'a ParsedTopic>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Collectors,
    Streams,
}

pub(crate) fn topic_browser(
    terminal: &mut DefaultTerminal,
    topics: &[String],
) -> Result<Option<String>> {
    let parsed: Vec<ParsedTopic> = topics.iter().filter_map(|t| parse_topic(t)).collect();

    // Group by collector
    let mut collector_map: BTreeMap<&str, Vec<&ParsedTopic>> = BTreeMap::new();
    for pt in &parsed {
        collector_map.entry(&pt.collector).or_default().push(pt);
    }
    let mut all_collectors: Vec<CollectorInfo> = collector_map
        .into_iter()
        .map(|(name, streams)| {
            let mut sorted = streams;
            sorted.sort_by_key(|pt| pt.asn_str.parse::<u32>().unwrap_or(0));
            let label = collector_label(name).to_string();
            CollectorInfo {
                raw_name: name.to_string(),
                label,
                stream_count: sorted.len(),
                streams: sorted,
            }
        })
        .collect();
    all_collectors.sort_by(|a, b| {
        let a_undef = a.raw_name.contains("UNDEFINED");
        let b_undef = b.raw_name.contains("UNDEFINED");
        a_undef
            .cmp(&b_undef)
            .then_with(|| b.stream_count.cmp(&a.stream_count))
    });

    let mut filter = String::new();
    let mut active_pane: Pane = Pane::Collectors;
    let mut collector_idx: usize = 0;
    let mut stream_idx: usize = 0;

    loop {
        let lower = filter.to_lowercase();
        let searching = !filter.is_empty();

        // Filter collectors: named ones always visible when not searching,
        // UNDEFINED only when searching. Collector label, raw name, or
        // any of its streams must match the search.
        let filtered_collectors: Vec<&CollectorInfo> = all_collectors
            .iter()
            .filter(|c| {
                let is_undefined = c.raw_name.contains("UNDEFINED");
                if is_undefined && !searching {
                    return false;
                }
                if !searching {
                    return true;
                }
                c.label.to_lowercase().contains(&lower)
                    || c.raw_name.to_lowercase().contains(&lower)
                    || c.streams.iter().any(|pt| {
                        pt.asn_str.contains(&lower)
                            || pt.full.to_lowercase().contains(&lower)
                            || as_name(&pt.asn_str).to_lowercase().contains(&lower)
                    })
            })
            .collect();

        // Clamp indices
        if collector_idx >= filtered_collectors.len().max(1) {
            collector_idx = filtered_collectors.len().saturating_sub(1);
            active_pane = Pane::Collectors;
        }

        // Streams for the currently selected collector, filtered by search
        let selected_collector_streams: Vec<&&ParsedTopic> =
            if let Some(ci) = filtered_collectors.get(collector_idx) {
                if searching {
                    ci.streams
                        .iter()
                        .filter(|pt| {
                            pt.asn_str.contains(&lower)
                                || pt.full.to_lowercase().contains(&lower)
                                || as_name(&pt.asn_str).to_lowercase().contains(&lower)
                                || ci.label.to_lowercase().contains(&lower)
                        })
                        .collect()
                } else {
                    ci.streams.iter().collect()
                }
            } else {
                Vec::new()
            };

        if stream_idx >= selected_collector_streams.len().max(1) {
            stream_idx = selected_collector_streams.len().saturating_sub(1);
        }

        // Selected topic for detail area
        let selected_topic = if filtered_collectors.is_empty() {
            None
        } else {
            selected_collector_streams
                .get(stream_idx)
                .map(|pt| pt.full.clone())
        };

        terminal.draw(|f| {
            let area = f.area();
            let chunks = Layout::vertical([
                Constraint::Length(3), // title
                Constraint::Length(3), // search
                Constraint::Min(0),    // body (2 panes)
                Constraint::Length(3), // selected detail + footer
            ])
            .split(area);

            // ── Title ──
            let subtitle = format!(
                "{} collectors  ·  {} streams",
                filtered_collectors.len(),
                parsed.len()
            );
            let title = Paragraph::new(vec![
                Line::from(" BMPWatch ").bold().centered(),
                Line::from(Span::styled(subtitle, Color::DarkGray)).centered(),
            ])
            .block(Block::bordered().borders(Borders::ALL));
            f.render_widget(title, chunks[0]);

            // ── Search ──
            let search_content: Line = if !searching {
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled("search", Color::DarkGray),
                    Span::raw("  —  type ASN, name, collector, or topic"),
                ])
            } else {
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("search  ▏{filter}"), Color::Reset),
                ])
            };
            f.render_widget(
                Paragraph::new(search_content).block(Block::bordered().borders(Borders::ALL)),
                chunks[1],
            );

            // ── Body: two panes ──
            let body = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[2]);

            // Left: Collectors
            let mut col_lines: Vec<Line> = Vec::new();
            for (i, c) in filtered_collectors.iter().enumerate() {
                let label_trunc = if c.label.len() > 30 {
                    format!("{}…", &c.label[..29])
                } else {
                    c.label.clone()
                };
                let text = format!(" {:<32} {:>4}", label_trunc, c.stream_count,);
                let on_collectors = active_pane == Pane::Collectors && i == collector_idx;
                let line = if on_collectors {
                    Line::from(text).on_white().black()
                } else {
                    Line::from(text)
                };
                col_lines.push(line);
            }
            if col_lines.is_empty() {
                col_lines.push(Line::from(" (no matches)").dark_gray());
            }
            f.render_widget(
                Paragraph::new(Text::from(col_lines))
                    .block(Block::bordered().title("Collectors").borders(Borders::ALL)),
                body[0],
            );

            // Right: Streams
            let mut stream_lines: Vec<Line> = Vec::new();
            for (i, pt) in selected_collector_streams.iter().enumerate() {
                let asn = format!("AS{}", pt.asn_str);
                let name = as_name(&pt.asn_str);
                let name_display = if name.len() > 24 {
                    format!("{}…", &name[..23])
                } else if name.is_empty() || name.starts_with("AS") {
                    String::new()
                } else {
                    name
                };
                let on_streams = active_pane == Pane::Streams && i == stream_idx;
                let line = if on_streams {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(asn, Color::Green),
                        Span::raw(" "),
                        Span::styled(name_display, Color::Yellow),
                    ])
                    .on_white()
                    .black()
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(asn, Color::Green),
                        Span::raw(" "),
                        if !name_display.is_empty() {
                            Span::styled(name_display, Color::Yellow)
                        } else {
                            Span::raw("")
                        },
                    ])
                };
                stream_lines.push(line);
            }
            if stream_lines.is_empty() {
                stream_lines.push(Line::from(" (no streams)").dark_gray());
            }
            let stream_title = if let Some(ci) = filtered_collectors.get(collector_idx) {
                format!("Streams — {}", ci.label)
            } else {
                "Streams".into()
            };
            f.render_widget(
                Paragraph::new(Text::from(stream_lines))
                    .block(Block::bordered().title(stream_title).borders(Borders::ALL)),
                body[1],
            );

            // ── Selected detail + footer ──
            let detail = selected_topic.as_deref().unwrap_or("no stream selected");
            let footer_lines = vec![
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled("↑↓", Color::White).bold(),
                    Span::raw(" move  "),
                    Span::styled("tab", Color::White).bold(),
                    Span::raw(" pane  "),
                    Span::styled("/", Color::White).bold(),
                    Span::raw(" search  "),
                    Span::styled("enter", Color::White).bold(),
                    Span::raw(" connect  "),
                    Span::styled("esc", Color::White).bold(),
                    Span::raw(" quit"),
                ]),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        detail,
                        if selected_topic.is_some() {
                            Color::Green
                        } else {
                            Color::DarkGray
                        },
                    ),
                    Span::raw("  "),
                    Span::styled("offline: bmpwatch <capture.bmpd>", Color::DarkGray),
                ]),
            ];
            f.render_widget(
                Paragraph::new(footer_lines).block(Block::bordered().borders(Borders::ALL)),
                chunks[3],
            );
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                    KeyCode::Tab => {
                        active_pane = match active_pane {
                            Pane::Collectors => Pane::Streams,
                            Pane::Streams => Pane::Collectors,
                        };
                    }
                    KeyCode::Enter => match active_pane {
                        Pane::Collectors => {
                            // Switch to streams for this collector
                            active_pane = Pane::Streams;
                            stream_idx = 0;
                        }
                        Pane::Streams => {
                            if let Some(pt) = selected_collector_streams.get(stream_idx) {
                                add_recent(&pt.full);
                                return Ok(Some(pt.full.clone()));
                            }
                        }
                    },
                    KeyCode::Char(c) => {
                        filter.push(c);
                        collector_idx = 0;
                        stream_idx = 0;
                        active_pane = Pane::Collectors;
                    }
                    KeyCode::Backspace => {
                        filter.pop();
                        collector_idx = 0;
                        stream_idx = 0;
                        active_pane = Pane::Collectors;
                    }
                    KeyCode::Up => match active_pane {
                        Pane::Collectors => {
                            collector_idx = collector_idx.saturating_sub(1);
                        }
                        Pane::Streams => {
                            stream_idx = stream_idx.saturating_sub(1);
                        }
                    },
                    KeyCode::Down => match active_pane {
                        Pane::Collectors => {
                            if collector_idx + 1 < filtered_collectors.len() {
                                collector_idx += 1;
                            }
                        }
                        Pane::Streams => {
                            if stream_idx + 1 < selected_collector_streams.len() {
                                stream_idx += 1;
                            }
                        }
                    },
                    _ => {}
                }
            }
        }
    }
}
