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
    // Split from the right: last segment is ASN, everything before is collector
    let (collector, asn_str) = match body.rsplit_once('.') {
        Some((col, asn)) => (col.to_string(), asn.to_string()),
        None => {
            // No dots — edge case: routeviews.<collector>.bmp_raw
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
        a_undef.cmp(&b_undef)
            .then_with(|| b.1.len().cmp(&a.1.len()))
    });

    // Build flat display: collector headers + stream rows
    enum Row {
        Header { collector: String, count: usize },
        Stream { topic: String, asn: String, name: String },
    }

    let mut filter = String::new();
    let mut selected: usize = 0;

    loop {
        let lower = filter.to_lowercase();
        let mut rows: Vec<Row> = Vec::new();

        for (col, streams) in &collectors {
            let mut matching: Vec<&&ParsedTopic> = if filter.is_empty() {
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
            matching.sort_by_key(|pt| {
                pt.asn_str.parse::<u32>().unwrap_or(0)
            });

            rows.push(Row::Header {
                collector: col.to_string(),
                count: matching.len(),
            });
            for pt in matching {
                rows.push(Row::Stream {
                    topic: pt.full.clone(),
                    asn: pt.asn_str.clone(),
                    name: as_name(&pt.asn_str),
                });
            }
        }

        if rows.is_empty() {
            rows.push(Row::Stream {
                topic: String::new(),
                asn: String::new(),
                name: String::new(),
            });
        }

        if selected >= rows.len() {
            selected = rows.len().saturating_sub(1);
        }

        terminal.draw(|f| {
            let area = f.area();
            let chunks = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);

            // Title
            let total = parsed.len();
            let title = Paragraph::new(format!(" BMPDoctor — {total} streams available "))
                .bold()
                .block(Block::bordered().borders(Borders::ALL))
                .centered();
            f.render_widget(title, chunks[0]);

            // Search bar
            let prompt: Line = if filter.is_empty() {
                Line::from(" type to search — ASN, name, collector, or topic ").dark_gray()
            } else {
                Line::from(format!(" {filter}"))
            };
            let search = Paragraph::new(prompt)
            .block(Block::bordered().borders(Borders::ALL));
            f.render_widget(search, chunks[1]);

            // Results
            let mut lines: Vec<Line> = Vec::new();
            let mut i = 0;
            for row in &rows {
                match row {
                    Row::Header { collector, count } => {
                        let text = format!("  {collector} ({count})");
                        let line = if i == selected {
                            Line::from(text).bold()
                        } else {
                            Line::from(text).dark_gray()
                        };
                        lines.push(line);
                    }
                    Row::Stream { topic, asn, name } => {
                        if topic.is_empty() {
                            lines.push(Line::from("  no matches").dark_gray());
                        } else {
                            let display = format!(
                                "    AS{:<8} {:<24} {}",
                                asn,
                                if name.len() > 24 { &name[..23] } else { name },
                                topic,
                            );
                            let line = if i == selected {
                                Line::from(display).on_white().black()
                            } else {
                                Line::from(display)
                            };
                            lines.push(line);
                        }
                    }
                }
                i += 1;
            }

            f.render_widget(
                Paragraph::new(Text::from(lines))
                    .block(Block::bordered().borders(Borders::ALL)),
                chunks[2],
            );

            // Footer
            let footer = format!(
                " [↑↓] navigate  [type] search  [enter] connect  [esc] quit  {} results",
                rows.len(),
            );
            f.render_widget(
                Paragraph::new(footer)
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
                                    return Ok(Some(topic.clone()));
                                }
                                _ => {
                                    // On a header or empty row — jump to next stream
                                    for j in selected + 1..rows.len() {
                                        if matches!(&rows[j], Row::Stream { topic, .. } if !topic.is_empty()) {
                                            selected = j;
                                            break;
                                        }
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
                        if selected > 0 {
                            selected -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if selected + 1 < rows.len() {
                            selected += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
