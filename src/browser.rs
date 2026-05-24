use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::DefaultTerminal;

use crate::dashboard::as_name;

#[derive(Clone)]
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

// Recent connections cache
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

// ── BrowserState ──

#[derive(Clone)]
struct CollectorInfo {
    raw_name: String,
    label: String,
    stream_count: usize,
    streams: Vec<ParsedTopic>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pane {
    Collectors,
    Streams,
}

struct BrowserState {
    filter: String,
    all_collectors: Vec<CollectorInfo>,
    filtered_indices: Vec<usize>,
    selected_collector: Option<usize>,
    selected_stream: Option<usize>,
    active_pane: Pane,
}

impl BrowserState {
    fn new(topics: &[String]) -> Self {
        let parsed: Vec<ParsedTopic> = topics.iter().filter_map(|t| parse_topic(t)).collect();
        let mut collector_map: BTreeMap<String, Vec<ParsedTopic>> = BTreeMap::new();
        for pt in parsed {
            collector_map
                .entry(pt.collector.clone())
                .or_default()
                .push(pt);
        }
        let mut all_collectors: Vec<CollectorInfo> = collector_map
            .into_iter()
            .map(|(name, mut streams)| {
                streams.sort_by_key(|pt| pt.asn_str.parse::<u32>().unwrap_or(0));
                CollectorInfo {
                    raw_name: name.clone(),
                    label: collector_label(&name).to_string(),
                    stream_count: streams.len(),
                    streams,
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
        let mut state = BrowserState {
            filter: String::new(),
            all_collectors,
            filtered_indices: Vec::new(),
            selected_collector: None,
            selected_stream: None,
            active_pane: Pane::Collectors,
        };
        state.rebuild_filter();
        state
    }

    fn current_collector(&self) -> Option<&CollectorInfo> {
        self.selected_collector
            .and_then(|i| self.filtered_indices.get(i))
            .map(|&idx| &self.all_collectors[idx])
    }

    fn current_streams(&self) -> &[ParsedTopic] {
        match self.current_collector() {
            Some(ci) => &ci.streams,
            None => &[],
        }
    }

    fn selected_topic(&self) -> Option<&str> {
        let streams = self.current_streams();
        match self.selected_stream {
            Some(i) if i < streams.len() => Some(streams[i].full.as_str()),
            _ => None,
        }
    }

    fn rebuild_filter(&mut self) {
        let lower = self.filter.to_lowercase();
        let searching = !self.filter.is_empty();

        self.filtered_indices = self
            .all_collectors
            .iter()
            .enumerate()
            .filter(|(_, ci)| {
                if !searching && ci.raw_name.contains("UNDEFINED") {
                    return false;
                }
                if !searching {
                    return true;
                }
                ci.label.to_lowercase().contains(&lower)
                    || ci.raw_name.to_lowercase().contains(&lower)
                    || ci.streams.iter().any(|pt| {
                        pt.asn_str.contains(&lower)
                            || pt.full.to_lowercase().contains(&lower)
                            || as_name(&pt.asn_str).to_lowercase().contains(&lower)
                    })
            })
            .map(|(i, _)| i)
            .collect();

        // Clamp collector selection
        self.selected_collector = match self.selected_collector {
            Some(i) if i < self.filtered_indices.len() => Some(i),
            _ if self.filtered_indices.is_empty() => None,
            _ => Some(0),
        };

        // Clamp stream selection
        let stream_len = self.current_streams().len();
        self.selected_stream = match self.selected_stream {
            Some(i) if i < stream_len => Some(i),
            _ if stream_len == 0 => None,
            _ => Some(0),
        };

        // If active pane has no items, switch
        if self.active_pane == Pane::Collectors && self.filtered_indices.is_empty() {
            self.active_pane = Pane::Streams;
        }
        if self.active_pane == Pane::Streams && self.current_streams().is_empty() {
            self.active_pane = Pane::Collectors;
        }
    }

    // ── Navigation ──

    fn move_up(&mut self) {
        match self.active_pane {
            Pane::Collectors => {
                self.selected_collector = match self.selected_collector {
                    Some(0) | None => self.selected_collector,
                    Some(i) => Some(i - 1),
                };
            }
            Pane::Streams => {
                self.selected_stream = match self.selected_stream {
                    Some(0) | None => self.selected_stream,
                    Some(i) => Some(i - 1),
                };
            }
        }
    }

    fn move_down(&mut self) {
        match self.active_pane {
            Pane::Collectors => {
                if let Some(i) = self.selected_collector {
                    if i + 1 < self.filtered_indices.len() {
                        self.selected_collector = Some(i + 1);
                    }
                }
            }
            Pane::Streams => {
                if let Some(i) = self.selected_stream {
                    if i + 1 < self.current_streams().len() {
                        self.selected_stream = Some(i + 1);
                    }
                }
            }
        }
    }

    fn switch_pane(&mut self) {
        match self.active_pane {
            Pane::Collectors => {
                if !self.current_streams().is_empty() {
                    self.active_pane = Pane::Streams;
                }
            }
            Pane::Streams => {
                if !self.filtered_indices.is_empty() {
                    self.active_pane = Pane::Collectors;
                }
            }
        }
    }

    fn type_char(&mut self, c: char) {
        self.filter.push(c);
        self.rebuild_filter();
    }

    fn backspace(&mut self) {
        self.filter.pop();
        self.rebuild_filter();
    }
}

// ── Viewport ──

/// Compute the visible range of items so the selected row is always on screen.
/// Returns `start..end` clamped to `[0, total]`.
fn visible_range(selected: Option<usize>, total: usize, height: u16) -> std::ops::Range<usize> {
    let height = height as usize;
    if total == 0 || height == 0 {
        return 0..0;
    }
    let window = height.max(1);
    let sel = selected.filter(|&s| s < total).unwrap_or(0);
    let start = sel
        .saturating_sub(window / 2)
        .min(total.saturating_sub(window));
    let end = (start + window).min(total);
    start..end
}

// ── Topic Browser ──

pub(crate) fn topic_browser(
    terminal: &mut DefaultTerminal,
    topics: &[String],
) -> Result<Option<String>> {
    let mut state = BrowserState::new(topics);

    loop {
        terminal.draw(|f| {
            let area = f.area();
            let chunks = Layout::vertical([
                Constraint::Length(3), // title
                Constraint::Length(3), // search
                Constraint::Min(0),    // body (2 panes)
                Constraint::Length(3), // detail + footer
            ])
            .split(area);

            // ── Title ──
            let subtitle = format!(
                "{} collectors  ·  {} streams",
                state.filtered_indices.len(),
                state
                    .all_collectors
                    .iter()
                    .map(|c| c.stream_count)
                    .sum::<usize>()
            );
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(" BMPWatch ").bold().centered(),
                    Line::from(Span::styled(subtitle, Color::DarkGray)).centered(),
                ])
                .block(Block::bordered().borders(Borders::ALL)),
                chunks[0],
            );

            // ── Search ──
            let search_content: Line = if state.filter.is_empty() {
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled("search", Color::DarkGray),
                    Span::raw("  —  type ASN, name, collector, or topic"),
                ])
            } else {
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("search  ▏{}", state.filter), Color::Reset),
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
            let col_height = body[0].height.saturating_sub(2); // subtract border
            let col_total = state.filtered_indices.len();
            let col_range = visible_range(state.selected_collector, col_total, col_height);
            let mut col_lines: Vec<Line> = Vec::new();
            for i in col_range {
                let idx = state.filtered_indices[i];
                let ci = &state.all_collectors[idx];
                let label_trunc = if ci.label.len() > 34 {
                    format!("{}…", &ci.label[..33])
                } else {
                    ci.label.clone()
                };
                let text = format!(" {:<38} {:>4}", label_trunc, ci.stream_count);
                let on =
                    state.active_pane == Pane::Collectors && state.selected_collector == Some(i);
                if on {
                    col_lines.push(Line::from(text).on_white().black());
                } else {
                    col_lines.push(Line::from(text));
                }
            }
            if col_lines.is_empty() {
                col_lines.push(Line::from(" (no matches)").dark_gray());
            }
            f.render_widget(
                Paragraph::new(ratatui::text::Text::from(col_lines))
                    .block(Block::bordered().title("Collectors").borders(Borders::ALL)),
                body[0],
            );

            // Right: Streams
            let stream_total = state.current_streams().len();
            let stream_height = body[1].height.saturating_sub(2);
            let stream_range = visible_range(state.selected_stream, stream_total, stream_height);
            let mut stream_lines: Vec<Line> = Vec::new();
            let streams = state.current_streams();
            for i in stream_range {
                let pt = &streams[i];
                let name = as_name(&pt.asn_str);
                let name_display = if name.len() > 26 {
                    format!("{}…", &name[..25])
                } else if name.is_empty() || name.starts_with("AS") {
                    String::new()
                } else {
                    name
                };
                let on = state.active_pane == Pane::Streams && state.selected_stream == Some(i);
                if on {
                    stream_lines.push(
                        Line::from(vec![
                            Span::raw("  "),
                            Span::styled(format!("AS{}", pt.asn_str), Color::Green),
                            Span::raw(" "),
                            Span::styled(name_display, Color::Yellow),
                        ])
                        .on_white()
                        .black(),
                    );
                } else {
                    stream_lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(format!("AS{}", pt.asn_str), Color::Green),
                        Span::raw(" "),
                        if !name_display.is_empty() {
                            Span::styled(name_display, Color::Yellow)
                        } else {
                            Span::raw("")
                        },
                    ]));
                }
            }
            if stream_lines.is_empty() {
                stream_lines.push(Line::from(" (no streams)").dark_gray());
            }
            let stream_title = match state.current_collector() {
                Some(ci) => format!("Streams — {}", ci.label),
                None => "Streams".into(),
            };
            f.render_widget(
                Paragraph::new(ratatui::text::Text::from(stream_lines))
                    .block(Block::bordered().title(stream_title).borders(Borders::ALL)),
                body[1],
            );

            // ── Selected detail + Footer ──
            let detail = state.selected_topic().unwrap_or("no stream selected");
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![
                        Span::raw(" "),
                        Span::styled("↑↓", Color::White).bold(),
                        Span::raw(" move  "),
                        Span::styled("tab", Color::White).bold(),
                        Span::raw(" pane  "),
                        Span::styled("enter", Color::White).bold(),
                        Span::raw(" connect  "),
                        Span::styled("esc", Color::White).bold(),
                        Span::raw(" quit"),
                    ]),
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            detail,
                            if state.selected_topic().is_some() {
                                Color::Green
                            } else {
                                Color::DarkGray
                            },
                        ),
                        Span::raw("  "),
                        Span::styled("offline: bmpwatch <capture.bmpd>", Color::DarkGray),
                    ]),
                ])
                .block(Block::bordered().borders(Borders::ALL)),
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
                    KeyCode::Tab => state.switch_pane(),
                    KeyCode::Enter => match state.active_pane {
                        Pane::Collectors => {
                            if !state.current_streams().is_empty() {
                                state.active_pane = Pane::Streams;
                                if state.selected_stream.is_none() {
                                    state.selected_stream = Some(0);
                                }
                            }
                        }
                        Pane::Streams => {
                            if let Some(topic) = state.selected_topic() {
                                add_recent(topic);
                                return Ok(Some(topic.to_string()));
                            }
                        }
                    },
                    KeyCode::Char(c) => state.type_char(c),
                    KeyCode::Backspace => state.backspace(),
                    KeyCode::Up => state.move_up(),
                    KeyCode::Down => state.move_down(),
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn test_topics() -> Vec<String> {
        vec![
            "routeviews.chicago.13335.bmp_raw".into(),
            "routeviews.chicago.2914.bmp_raw".into(),
            "routeviews.linx.13335.bmp_raw".into(),
            "routeviews.linx.3257.bmp_raw".into(),
            "routeviews.nwax.714.bmp_raw".into(),
            "routeviews.UNDEFINED_ROUTER_GROUP.99999.bmp_raw".into(),
        ]
    }

    // ── Navigation ──

    #[test]
    fn test_move_up_at_top_stays() {
        let mut s = BrowserState::new(&test_topics());
        s.move_up();
        assert_eq!(s.selected_collector, Some(0));
    }

    #[test]
    fn test_move_up_moves() {
        let mut s = BrowserState::new(&test_topics());
        s.move_down();
        s.move_down();
        assert_eq!(s.selected_collector, Some(2));
        s.move_up();
        assert_eq!(s.selected_collector, Some(1));
    }

    #[test]
    fn test_move_down_at_bottom_stays() {
        let mut s = BrowserState::new(&test_topics());
        let last = s.filtered_indices.len() - 1;
        for _ in 0..100 {
            s.move_down();
        }
        assert_eq!(s.selected_collector, Some(last));
    }

    #[test]
    fn test_move_down_from_none_stays_none() {
        let mut s = BrowserState::new(&[]);
        s.move_down();
        assert_eq!(s.selected_collector, None);
    }

    #[test]
    fn test_move_up_from_none_stays_none() {
        let mut s = BrowserState::new(&[]);
        s.move_up();
        assert_eq!(s.selected_collector, None);
    }

    #[test]
    fn test_switch_pane_to_empty_stays() {
        let mut s = BrowserState::new(&[]);
        s.switch_pane();
        assert_eq!(s.active_pane, Pane::Collectors);
    }

    // ── Filtering ──

    #[test]
    fn test_empty_filter_shows_named_collectors() {
        let s = BrowserState::new(&test_topics());
        // 3 named collectors (chicago, linx, nwax), UNDEFINED hidden
        assert_eq!(s.filtered_indices.len(), 3);
        // Check none are UNDEFINED
        for &idx in &s.filtered_indices {
            assert!(!s.all_collectors[idx].raw_name.contains("UNDEFINED"));
        }
    }

    #[test]
    fn test_empty_filter_hides_undefined() {
        let s = BrowserState::new(&test_topics());
        let has_undef = s
            .filtered_indices
            .iter()
            .any(|&i| s.all_collectors[i].raw_name.contains("UNDEFINED"));
        assert!(!has_undef);
    }

    #[test]
    fn test_filter_matches() {
        let mut s = BrowserState::new(&test_topics());
        s.type_char('1');
        s.type_char('3');
        s.type_char('3');
        s.type_char('3');
        s.type_char('5');
        // Should match chicago (13335, 2914) and linx (13335, 3257) — at least some results
        assert!(!s.filtered_indices.is_empty());
        assert!(s.selected_collector.is_some());
    }

    #[test]
    fn test_filter_nonexistent_clears_selections() {
        let mut s = BrowserState::new(&test_topics());
        s.type_char('z');
        s.type_char('z');
        s.type_char('z');
        assert!(s.filtered_indices.is_empty());
        assert_eq!(s.selected_collector, None);
        assert_eq!(s.selected_topic(), None);
    }

    #[test]
    fn test_backspace_restores_list() {
        let mut s = BrowserState::new(&test_topics());
        let before = s.filtered_indices.len();
        s.type_char('z');
        s.type_char('z');
        s.type_char('z');
        assert!(s.filtered_indices.is_empty());
        s.backspace();
        s.backspace();
        s.backspace();
        assert_eq!(s.filtered_indices.len(), before);
    }

    // ── Viewport ──

    #[test]
    fn test_visible_range_few_in_tall_window() {
        let r = visible_range(Some(2), 5, 20);
        assert_eq!(r, 0..5);
    }

    #[test]
    fn test_visible_range_many_selected_middle() {
        let r = visible_range(Some(50), 100, 10);
        assert!(r.contains(&50));
        assert_eq!(r.len(), 10);
    }

    #[test]
    fn test_visible_range_none_empty() {
        let r = visible_range(None, 0, 10);
        assert_eq!(r, 0..0);
    }

    #[test]
    fn test_visible_range_selected_bottom() {
        let r = visible_range(Some(99), 100, 10);
        assert!(r.contains(&99));
        assert_eq!(r.len(), 10);
        assert_eq!(r.end, 100);
    }

    #[test]
    fn test_selected_topic_none_on_empty() {
        let s = BrowserState::new(&[]);
        assert_eq!(s.selected_topic(), None);
    }

    #[test]
    fn test_selected_topic_returns_correct() {
        let s = BrowserState::new(&test_topics());
        let topic = s.selected_topic();
        assert!(topic.is_some());
        assert!(topic.unwrap().contains(".bmp_raw"));
    }

    // ── Render ──

    #[test]
    fn test_render_no_panic_80x24() {
        let topics = test_topics();
        let state = BrowserState::new(&topics);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        // Replicate the render block minimally
        terminal
            .draw(|f| {
                let chunks = Layout::vertical([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(0),
                    Constraint::Length(3),
                ])
                .split(f.area());
                let body =
                    Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                        .split(chunks[2]);
                let col_total = state.filtered_indices.len();
                let col_h = body[0].height.saturating_sub(2);
                let col_range = visible_range(state.selected_collector, col_total, col_h);
                let mut col_lines: Vec<Line> = Vec::new();
                for i in col_range {
                    let ci = &state.all_collectors[state.filtered_indices[i]];
                    col_lines.push(Line::from(format!(" {}  ({})", ci.label, ci.stream_count)));
                }
                f.render_widget(
                    Paragraph::new(ratatui::text::Text::from(col_lines))
                        .block(Block::bordered().title("Collectors").borders(Borders::ALL)),
                    body[0],
                );

                let streams = state.current_streams();
                let str_h = body[1].height.saturating_sub(2);
                let str_range = visible_range(state.selected_stream, streams.len(), str_h);
                let mut stream_lines: Vec<Line> = Vec::new();
                for i in str_range {
                    stream_lines.push(Line::from(format!(
                        "  AS{}  {}",
                        streams[i].asn_str, streams[i].full
                    )));
                }
                f.render_widget(
                    Paragraph::new(ratatui::text::Text::from(stream_lines))
                        .block(Block::bordered().title("Streams").borders(Borders::ALL)),
                    body[1],
                );
            })
            .expect("render should not panic");
    }

    #[test]
    fn test_render_no_panic_100x30() {
        let topics = test_topics();
        let state = BrowserState::new(&topics);
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let chunks = Layout::vertical([
                    Constraint::Length(3),
                    Constraint::Min(0),
                    Constraint::Length(3),
                ])
                .split(f.area());
                let body =
                    Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                        .split(chunks[1]);
                f.render_widget(
                    Paragraph::new(format!("{} collectors", state.filtered_indices.len()))
                        .block(Block::bordered().borders(Borders::ALL)),
                    body[0],
                );
                f.render_widget(
                    Paragraph::new(format!("{} streams", state.current_streams().len()))
                        .block(Block::bordered().borders(Borders::ALL)),
                    body[1],
                );
            })
            .expect("render should not panic");
    }

    // ── Scroll endurance ──

    #[test]
    fn test_scroll_endurance() {
        // Simulate hundreds of down/up presses without panic
        let mut s = BrowserState::new(&test_topics());
        for _ in 0..500 {
            s.move_down();
            s.move_up();
        }
        // Should still have valid selection
        assert!(s.selected_collector.is_some());
    }
}
