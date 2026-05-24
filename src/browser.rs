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

fn get_recents() -> Vec<String> {
    recent_cache().lock().unwrap().clone()
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

pub(crate) fn topic_browser(
    terminal: &mut DefaultTerminal,
    topics: &[String],
) -> Result<Option<String>> {
    let parsed: Vec<ParsedTopic> = topics.iter().filter_map(|t| parse_topic(t)).collect();

    // Group by collector, sorted by stream count desc
    let mut collector_map: BTreeMap<&str, Vec<&ParsedTopic>> = BTreeMap::new();
    for pt in &parsed {
        collector_map.entry(&pt.collector).or_default().push(pt);
    }
    let mut collectors: Vec<(&str, Vec<&ParsedTopic>)> = collector_map.into_iter().collect();
    collectors.sort_by(|a, b| {
        let a_undef = a.0.contains("UNDEFINED");
        let b_undef = b.0.contains("UNDEFINED");
        a_undef
            .cmp(&b_undef)
            .then_with(|| b.1.len().cmp(&a.1.len()))
    });

    // Index parsed topics by full name for recent lookup
    let topic_map: std::collections::HashMap<&str, &ParsedTopic> =
        parsed.iter().map(|pt| (pt.full.as_str(), pt)).collect();

    enum Row {
        Header {
            collector: String,
            count: usize,
        },
        Stream {
            topic: String,
            asn: String,
            name: String,
        },
        Empty,
    }

    let mut filter = String::new();
    let mut selected: usize = 0;
    let collector_count = collectors.len();

    loop {
        let lower = filter.to_lowercase();
        let recents = get_recents();
        let mut rows: Vec<Row> = Vec::new();

        // Recent section (only when not filtering)
        if filter.is_empty() && !recents.is_empty() {
            let recent_items: Vec<&&ParsedTopic> = recents
                .iter()
                .filter_map(|t| topic_map.get(t.as_str()))
                .take(5)
                .collect();
            if !recent_items.is_empty() {
                rows.push(Row::Header {
                    collector: "Recently Connected".into(),
                    count: recent_items.len(),
                });
                for pt in recent_items {
                    rows.push(Row::Stream {
                        topic: pt.full.clone(),
                        asn: pt.asn_str.clone(),
                        name: as_name(&pt.asn_str),
                    });
                }
                rows.push(Row::Empty);
            }
        }

        // All collectors. UNDEFINED_ROUTER_GROUP hidden by default;
        // shown only when the user's search filter matches its topics.
        for (col, streams) in &collectors {
            // Hide unnamed collectors unless the user is searching within them
            let is_undefined = col.contains("UNDEFINED");
            if is_undefined && filter.is_empty() {
                continue;
            }
            let matching: Vec<&&ParsedTopic> = if filter.is_empty() {
                streams.iter().collect()
            } else {
                streams
                    .iter()
                    .filter(|pt| {
                        pt.collector.to_lowercase().contains(&lower)
                            || pt.asn_str.contains(&lower)
                            || pt.full.to_lowercase().contains(&lower)
                            || as_name(&pt.asn_str).to_lowercase().contains(&lower)
                    })
                    .collect()
            };
            if matching.is_empty() {
                continue;
            }
            let mut sorted = matching;
            sorted.sort_by_key(|pt| pt.asn_str.parse::<u32>().unwrap_or(0));

            rows.push(Row::Header {
                collector: collector_label(col).to_string(),
                count: sorted.len(),
            });
            for pt in sorted {
                rows.push(Row::Stream {
                    topic: pt.full.clone(),
                    asn: pt.asn_str.clone(),
                    name: as_name(&pt.asn_str),
                });
            }
        }

        if rows.is_empty() {
            rows.push(Row::Empty);
        }

        if selected >= rows.len() {
            selected = rows.len().saturating_sub(1);
        }

        terminal.draw(|f| {
            let area = f.area();
            let chunks = Layout::vertical([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(2),
            ])
            .split(area);

            // ── Title ──
            let subtitle = if filter.is_empty() {
                format!(
                    "{} collectors  ·  {} streams",
                    collector_count,
                    parsed.len()
                )
            } else {
                format!(
                    "{} results",
                    rows.iter()
                        .filter(|r| matches!(r, Row::Stream { .. }))
                        .count()
                )
            };
            let title = Paragraph::new(vec![
                Line::from(" BMPWatch ").bold().centered(),
                Line::from(Span::styled(subtitle, Color::DarkGray)).centered(),
            ])
            .block(Block::bordered().borders(Borders::ALL));
            f.render_widget(title, chunks[0]);

            // ── Search ──
            let search_content: Line = if filter.is_empty() {
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

            // ── Results ──
            let mut lines: Vec<Line> = Vec::new();
            for (i, row) in rows.iter().enumerate() {
                match row {
                    Row::Header { collector, count } => {
                        let text = format!(" {collector}  ({count})");
                        let line = if i == selected {
                            Line::from(text).cyan().bold()
                        } else {
                            Line::from(text).cyan()
                        };
                        lines.push(line);
                    }
                    Row::Stream { topic, asn, name } => {
                        let asn_num = format!("AS{asn:<7}");
                        let name_display = if name.len() > 22 {
                            format!("{}…", &name[..21])
                        } else {
                            name.clone()
                        };
                        let has_name = !name.is_empty() && !name.starts_with("AS");
                        let line = if i == selected {
                            Line::from(vec![
                                Span::raw("   "),
                                Span::styled(asn_num, Color::Green),
                                Span::raw(" "),
                                if has_name {
                                    Span::styled(name_display, Color::Yellow)
                                } else {
                                    Span::raw(format!("{:<23}", name_display))
                                },
                                Span::raw(" "),
                                Span::raw(topic.as_str()),
                            ])
                            .on_white()
                            .black()
                        } else {
                            Line::from(vec![
                                Span::raw("   "),
                                Span::styled(asn_num, Color::Green),
                                Span::raw(" "),
                                if has_name {
                                    Span::styled(name_display, Color::Yellow)
                                } else {
                                    Span::raw(format!("{:<23}", name_display))
                                },
                                Span::raw(" "),
                                Span::styled(topic.as_str(), Color::DarkGray),
                            ])
                        };
                        lines.push(line);
                    }
                    Row::Empty => {
                        lines.push(Line::from(vec![Span::raw("")]));
                    }
                }
            }

            f.render_widget(
                Paragraph::new(Text::from(lines)).block(Block::bordered().borders(Borders::ALL)),
                chunks[2],
            );

            // ── Footer ──
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![
                        Span::raw(" "),
                        Span::styled("↑↓", Color::White).bold(),
                        Span::raw(" navigate  "),
                        Span::styled("type", Color::White).bold(),
                        Span::raw(" filter  "),
                        Span::styled("enter", Color::White).bold(),
                        Span::raw(" connect  "),
                        Span::styled("esc", Color::White).bold(),
                        Span::raw(" quit"),
                    ]),
                    Line::from(Span::styled(
                        " offline: bmpwatch <capture.bmpd>",
                        Color::DarkGray,
                    )),
                ])
                .block(Block::bordered().borders(Borders::ALL))
                .centered(),
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
                    KeyCode::Enter => {
                        if let Some(row) = rows.get(selected) {
                            match row {
                                Row::Stream { topic, .. } if !topic.is_empty() => {
                                    add_recent(topic);
                                    return Ok(Some(topic.clone()));
                                }
                                _ => {
                                    if let Some(pos) = rows
                                        .iter()
                                        .enumerate()
                                        .skip(selected + 1)
                                        .position(|(_, r)| matches!(r, Row::Stream { topic, .. } if !topic.is_empty()))
                                    {
                                        selected = selected + 1 + pos;
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Char(c) => {
                        filter.push(c);
                        selected = 0;
                    }
                    KeyCode::Backspace => {
                        filter.pop();
                        selected = 0;
                    }
                    KeyCode::Up => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyCode::Down if selected + 1 < rows.len() => {
                        selected += 1;
                    }
                    _ => {}
                }
            }
        }
    }
}
