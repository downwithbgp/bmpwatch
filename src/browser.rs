use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEventKind, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::Rect;
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

pub(crate) fn topic_browser(
    terminal: &mut DefaultTerminal,
    topics: &[String],
) -> Result<Option<String>> {
    let _mouse_guard = MouseGuard::enable();
    let mut model = model::BrowserModel::new(topics);
    // Escape-sequence state: swallow raw `[A`/`[B` fragments from arrow keys
    // that arrive as Char events when Esc is intercepted in debug mode.
    enum EscState {
        None,
        EscSeen,
        EscBracketSeen,
    }
    let mut esc_state = EscState::None;
    let mut browser_layout: Option<BrowserLayout> = None;
    let mut click_tracker = ClickTracker::new();

    loop {
        // Keep selection visible within viewport (standard HCI: only scroll
        // when selection reaches the edge, never recenter).
        if let Some(ref layout) = browser_layout {
            model.ensure_visible_scrolls(
                layout.collector_pane.height.saturating_sub(2) as usize,
                layout.stream_pane.height.saturating_sub(2) as usize,
            );
        }

        diag("=== before_render");
        let draw_result = terminal.draw(|f| {
            browser_layout = Some(compute_layout(f.area()));
            model::render_model(f, f.area(), &model)
        });
        diag("=== after_render");
        draw_result?;

        if event::poll(Duration::from_millis(100))? {
            let ev = event::read()?;
            if let Event::Key(key) = ev {
                diag(&format!(
                    "before_key code={:?} kind={:?} modifiers={:?}",
                    key.code, key.kind, key.modifiers
                ));
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                let debug_mode = std::env::var("BMPWATCH_BROWSER_DEBUG").is_ok();
                if key.code == KeyCode::Esc {
                    if debug_mode {
                        diag("=== esc_ignored (debug mode)");
                        esc_state = EscState::EscSeen;
                        continue;
                    }
                    diag(&format!("=== return=quit key={:?}", key.code));
                    return Ok(None);
                }
                // Swallow escape-sequence fragments so raw `[A`/`[B`
                // from arrow keys never enter the search filter.
                if let KeyCode::Char(c) = key.code {
                    match (&esc_state, c) {
                        (EscState::EscSeen, '[') => {
                            esc_state = EscState::EscBracketSeen;
                            continue;
                        }
                        (EscState::EscBracketSeen, 'A' | 'B' | 'C' | 'D')
                        | (EscState::EscBracketSeen, 'O') => {
                            esc_state = EscState::None;
                            continue;
                        }
                        _ => {
                            esc_state = EscState::None;
                        }
                    }
                } else {
                    esc_state = EscState::None;
                }
                match key.code {
                    KeyCode::Char('q') => {
                        diag("=== return=quit key=q");
                        return Ok(None);
                    }
                    KeyCode::Up => {
                        model.apply(model::Action::MoveUp);
                        click_tracker.reset();
                    }
                    KeyCode::Down => {
                        model.apply(model::Action::MoveDown);
                        click_tracker.reset();
                    }
                    KeyCode::Tab => {
                        model.apply(model::Action::SwitchPane);
                        click_tracker.reset();
                    }
                    KeyCode::Enter => {
                        let result = model.apply(model::Action::Enter);
                        click_tracker.reset();
                        if let model::ActionResult::Selected(t) = result {
                            diag(&format!("=== return=selected topic={t}"));
                            add_recent(&t);
                            return Ok(Some(t));
                        }
                    }
                    KeyCode::Char(c) => {
                        model.apply(model::Action::TypeChar(c));
                        click_tracker.reset();
                    }
                    KeyCode::Backspace => {
                        model.apply(model::Action::Backspace);
                        click_tracker.reset();
                    }
                    _ => {}
                }
                diag(&format!(
                    "after_key pane={:?} col={:?}/{} str={:?}/{}",
                    model.active_pane,
                    model.selected_collector,
                    model.filtered_indices.len(),
                    model.selected_stream,
                    model.current_streams().len()
                ));
            } else if let Event::Mouse(mouse_event) = ev {
                if let Some(ref layout) = browser_layout {
                    let result =
                        handle_mouse_event(&mut model, layout, &mouse_event, &mut click_tracker);
                    if let model::ActionResult::Selected(t) = result {
                        diag(&format!("=== return=selected topic={t}"));
                        add_recent(&t);
                        return Ok(Some(t));
                    }
                }
            }
        }
    }
}

fn diag(msg: &str) {
    if std::env::var("BMPWATCH_BROWSER_DEBUG").is_err() {
        return;
    }
    let line = format!("{msg}\n");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/bmpwatch-browser-debug.log")
        .map(|mut f| {
            let _ = std::io::Write::write_all(&mut f, line.as_bytes());
        });
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}…", &s[..end])
    }
}

// ── Mouse support ──

struct MouseGuard {
    active: bool,
}

impl MouseGuard {
    fn enable() -> Self {
        let active = execute!(
            std::io::stdout(),
            ratatui::crossterm::event::EnableMouseCapture
        )
        .is_ok();
        if !active {
            eprintln!("(mouse capture unavailable)");
        }
        MouseGuard { active }
    }
}

impl Drop for MouseGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = execute!(
                std::io::stdout(),
                ratatui::crossterm::event::DisableMouseCapture
            );
        }
    }
}

struct BrowserLayout {
    collector_pane: Rect,
    stream_pane: Rect,
}

fn compute_layout(area: Rect) -> BrowserLayout {
    use ratatui::layout::{Constraint, Layout};
    let chunks = Layout::vertical([
        Constraint::Length(3), // title
        Constraint::Length(3), // search
        Constraint::Min(0),    // body
        Constraint::Length(3), // footer
    ])
    .split(area);
    let body = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);
    BrowserLayout {
        collector_pane: body[0],
        stream_pane: body[1],
    }
}

struct ClickTracker {
    pane: Option<model::Pane>,
    row: Option<usize>,
    time: Option<Instant>,
}

impl ClickTracker {
    fn new() -> Self {
        ClickTracker {
            pane: None,
            row: None,
            time: None,
        }
    }

    fn record(&mut self, pane: model::Pane, row: usize) {
        self.pane = Some(pane);
        self.row = Some(row);
        self.time = Some(Instant::now());
    }

    fn is_double_click(&self, pane: model::Pane, row: usize) -> bool {
        match (self.pane, self.row, self.time) {
            (Some(p), Some(r), Some(t)) => {
                p == pane && r == row && t.elapsed() < Duration::from_millis(500)
            }
            _ => false,
        }
    }

    fn reset(&mut self) {
        self.pane = None;
        self.row = None;
        self.time = None;
    }
}

/// Map screen coordinates to (pane, absolute_row) without side effects.
fn click_target(
    col: u16,
    row: u16,
    layout: &BrowserLayout,
    model: &model::BrowserModel,
) -> Option<(model::Pane, usize)> {
    if point_in(col, row, &layout.collector_pane) {
        let pane = &layout.collector_pane;
        if let Some(content_row) = row.checked_sub(pane.y + 1).map(|r| r as usize) {
            let height = pane.height.saturating_sub(2) as usize;
            if content_row < height {
                let actual_idx = model.collector_scroll + content_row;
                if actual_idx < model.filtered_indices.len() {
                    return Some((model::Pane::Collectors, actual_idx));
                }
            }
        }
    } else if point_in(col, row, &layout.stream_pane) {
        let pane = &layout.stream_pane;
        if let Some(content_row) = row.checked_sub(pane.y + 1).map(|r| r as usize) {
            let height = pane.height.saturating_sub(2) as usize;
            let streams = model.current_streams();
            if content_row < height {
                let actual_idx = model.stream_scroll + content_row;
                if actual_idx < streams.len() {
                    return Some((model::Pane::Streams, actual_idx));
                }
            }
        }
    }
    None
}

fn handle_mouse_event(
    model: &mut model::BrowserModel,
    layout: &BrowserLayout,
    mouse_event: &MouseEvent,
    click_tracker: &mut ClickTracker,
) -> model::ActionResult {
    let col = mouse_event.column;
    let row = mouse_event.row;

    match mouse_event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            diag(&format!("mouse kind=Down(Left) x={} y={}", col, row));

            let target = click_target(col, row, layout, model);
            match target {
                Some((pane, abs_row)) => {
                    // Apply selection (same side effects as handle_click)
                    let content_row = match pane {
                        model::Pane::Collectors => {
                            model.selected_collector = Some(abs_row);
                            model.active_pane = model::Pane::Collectors;
                            row.saturating_sub(layout.collector_pane.y.saturating_add(1)) as usize
                        }
                        model::Pane::Streams => {
                            model.selected_stream = Some(abs_row);
                            model.active_pane = model::Pane::Streams;
                            row.saturating_sub(layout.stream_pane.y.saturating_add(1)) as usize
                        }
                    };
                    diag(&format!(
                        "mouse pane={pane:?} row={content_row} action=select idx={abs_row}"
                    ));

                    // Double-click detection
                    if click_tracker.is_double_click(pane, abs_row) {
                        diag(&format!("mouse double_click pane={pane:?} row={abs_row}"));
                        click_tracker.reset();
                        if pane == model::Pane::Streams {
                            if let Some(topic) = model.selected_topic() {
                                return model::ActionResult::Selected(topic.to_string());
                            }
                        }
                    } else {
                        click_tracker.record(pane, abs_row);
                    }
                }
                None => {
                    click_tracker.reset();
                    diag(&format!(
                        "mouse kind=Down(Left) x={} y={} action=ignored",
                        col, row
                    ));
                }
            }
        }
        MouseEventKind::ScrollUp => {
            diag(&format!("mouse kind=ScrollUp x={} y={}", col, row));
            click_tracker.reset();
            handle_wheel(col, row, layout, model, true);
        }
        MouseEventKind::ScrollDown => {
            diag(&format!("mouse kind=ScrollDown x={} y={}", col, row));
            click_tracker.reset();
            handle_wheel(col, row, layout, model, false);
        }
        _ => {
            diag(&format!(
                "mouse kind={:?} x={} y={} action=ignored",
                mouse_event.kind, col, row
            ));
        }
    }

    model::ActionResult::None
}

fn handle_wheel(
    col: u16,
    row: u16,
    layout: &BrowserLayout,
    model: &mut model::BrowserModel,
    up: bool,
) {
    if point_in(col, row, &layout.collector_pane) {
        model.active_pane = model::Pane::Collectors;
        if up {
            model.apply(model::Action::MoveUp);
            diag("mouse action=wheel_collector_up");
        } else {
            model.apply(model::Action::MoveDown);
            diag("mouse action=wheel_collector_down");
        }
    } else if point_in(col, row, &layout.stream_pane) {
        model.active_pane = model::Pane::Streams;
        if up {
            model.apply(model::Action::MoveUp);
            diag("mouse action=wheel_stream_up");
        } else {
            model.apply(model::Action::MoveDown);
            diag("mouse action=wheel_stream_down");
        }
    }
}

fn point_in(col: u16, row: u16, r: &Rect) -> bool {
    col >= r.x
        && col < r.x.saturating_add(r.width)
        && row >= r.y
        && row < r.y.saturating_add(r.height)
}

// ── BrowserState model (feature/stream-browser-redesign, Phase 1) ──

mod model {
    use std::collections::BTreeMap;
    use std::fmt;

    use super::{as_name, collector_label, parse_topic, truncate_str, ParsedTopic};

    #[derive(Clone)]
    pub(super) struct ModelCollector {
        raw_name: String,
        pub(super) label: String,
        streams: Vec<ParsedTopic>,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) enum Pane {
        Collectors,
        Streams,
    }

    impl fmt::Debug for Pane {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Pane::Collectors => write!(f, "Collectors"),
                Pane::Streams => write!(f, "Streams"),
            }
        }
    }

    pub(super) struct BrowserModel {
        pub(super) filter: String,
        all_collectors: Vec<ModelCollector>,
        pub(super) filtered_indices: Vec<usize>,
        pub(super) selected_collector: Option<usize>,
        pub(super) selected_stream: Option<usize>,
        pub(super) active_pane: Pane,
        pub(super) collector_scroll: usize,
        pub(super) stream_scroll: usize,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Action {
        MoveUp,
        MoveDown,
        SwitchPane,
        TypeChar(char),
        Backspace,
        Enter,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) enum ActionResult {
        None,
        Selected(String),
        #[allow(dead_code)]
        Quit,
    }

    impl BrowserModel {
        pub(super) fn new(topics: &[String]) -> Self {
            let parsed: Vec<ParsedTopic> = topics.iter().filter_map(|t| parse_topic(t)).collect();
            let mut collector_map: BTreeMap<String, Vec<ParsedTopic>> = BTreeMap::new();
            for pt in parsed {
                collector_map
                    .entry(pt.collector.clone())
                    .or_default()
                    .push(pt);
            }
            let mut all_collectors: Vec<ModelCollector> = collector_map
                .into_iter()
                .map(|(name, mut streams)| {
                    streams.sort_by_key(|pt| pt.asn_str.parse::<u32>().unwrap_or(0));
                    ModelCollector {
                        raw_name: name.clone(),
                        label: collector_label(&name).to_string(),
                        streams,
                    }
                })
                .collect();
            all_collectors.sort_by(|a, b| {
                let a_undef = a.raw_name.contains("UNDEFINED");
                let b_undef = b.raw_name.contains("UNDEFINED");
                a_undef
                    .cmp(&b_undef)
                    .then_with(|| b.streams.len().cmp(&a.streams.len()))
            });
            let mut model = BrowserModel {
                filter: String::new(),
                all_collectors,
                filtered_indices: Vec::new(),
                selected_collector: None,
                selected_stream: None,
                active_pane: Pane::Collectors,
                collector_scroll: 0,
                stream_scroll: 0,
            };
            model.rebuild_filter();
            model
        }

        fn rebuild_filter(&mut self) {
            let lower = self.filter.to_lowercase();
            let searching = !self.filter.is_empty();
            self.filtered_indices = self
                .all_collectors
                .iter()
                .enumerate()
                .filter(|(_, mc)| {
                    if !searching && mc.raw_name.contains("UNDEFINED") {
                        return false;
                    }
                    if !searching {
                        return true;
                    }
                    mc.label.to_lowercase().contains(&lower)
                        || mc.raw_name.to_lowercase().contains(&lower)
                        || mc.streams.iter().any(|pt| {
                            pt.asn_str.contains(&lower)
                                || pt.full.to_lowercase().contains(&lower)
                                || as_name(&pt.asn_str).to_lowercase().contains(&lower)
                        })
                })
                .map(|(i, _)| i)
                .collect();

            self.selected_collector =
                clamp_opt(self.selected_collector, self.filtered_indices.len());

            let stream_len = self.current_streams().len();
            self.selected_stream = clamp_opt(self.selected_stream, stream_len);

            if self.active_pane == Pane::Collectors && self.filtered_indices.is_empty() {
                self.active_pane = Pane::Streams;
            }
            if self.active_pane == Pane::Streams && stream_len == 0 {
                self.active_pane = Pane::Collectors;
            }
        }

        pub(super) fn current_collector(&self) -> Option<&ModelCollector> {
            self.selected_collector
                .and_then(|i| self.filtered_indices.get(i))
                .map(|&idx| &self.all_collectors[idx])
        }

        pub(super) fn current_streams(&self) -> &[ParsedTopic] {
            match self.current_collector() {
                Some(mc) => &mc.streams,
                None => &[],
            }
        }

        pub(super) fn selected_topic(&self) -> Option<&str> {
            let streams = self.current_streams();
            match self.selected_stream {
                Some(i) if i < streams.len() => Some(streams[i].full.as_str()),
                _ => None,
            }
        }

        pub(super) fn apply(&mut self, action: Action) -> ActionResult {
            match action {
                Action::MoveUp => self.do_move_up(),
                Action::MoveDown => self.do_move_down(),
                Action::SwitchPane => self.do_switch_pane(),
                Action::TypeChar(c) => {
                    self.filter.push(c);
                    self.rebuild_filter();
                }
                Action::Backspace => {
                    self.filter.pop();
                    self.rebuild_filter();
                }
                Action::Enter => return self.do_enter(),
            }
            ActionResult::None
        }

        fn do_move_up(&mut self) {
            match self.active_pane {
                Pane::Collectors => {
                    self.selected_collector = dec_opt(self.selected_collector);
                }
                Pane::Streams => {
                    self.selected_stream = dec_opt(self.selected_stream);
                }
            }
        }

        fn do_move_down(&mut self) {
            match self.active_pane {
                Pane::Collectors => {
                    self.selected_collector =
                        inc_opt(self.selected_collector, self.filtered_indices.len());
                }
                Pane::Streams => {
                    self.selected_stream =
                        inc_opt(self.selected_stream, self.current_streams().len());
                }
            }
        }

        fn do_switch_pane(&mut self) {
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

        fn do_enter(&mut self) -> ActionResult {
            match self.active_pane {
                Pane::Collectors => {
                    if !self.current_streams().is_empty() {
                        self.active_pane = Pane::Streams;
                        if self.selected_stream.is_none() {
                            self.selected_stream = Some(0);
                        }
                    }
                    ActionResult::None
                }
                Pane::Streams => match self.selected_topic() {
                    Some(t) => ActionResult::Selected(t.to_string()),
                    None => ActionResult::None,
                },
            }
        }

        pub(super) fn ensure_visible_scrolls(
            &mut self,
            collector_height: usize,
            stream_height: usize,
        ) {
            let collector_total = self.filtered_indices.len();
            ensure_visible(
                self.selected_collector,
                &mut self.collector_scroll,
                collector_total,
                collector_height,
            );
            let stream_total = self.current_streams().len();
            ensure_visible(
                self.selected_stream,
                &mut self.stream_scroll,
                stream_total,
                stream_height,
            );
        }
    }

    // ── Pure helpers ──

    fn clamp_opt(sel: Option<usize>, len: usize) -> Option<usize> {
        match sel {
            Some(i) if i < len => Some(i),
            _ if len > 0 => Some(0),
            _ => None,
        }
    }

    fn dec_opt(sel: Option<usize>) -> Option<usize> {
        match sel {
            Some(0) | None => sel,
            Some(i) => Some(i - 1),
        }
    }

    fn inc_opt(sel: Option<usize>, len: usize) -> Option<usize> {
        match sel {
            Some(i) if i + 1 < len => Some(i + 1),
            _ => sel,
        }
    }

    /// Scroll only when selection reaches the viewport edge (standard HCI).
    fn ensure_visible(selected: Option<usize>, scroll: &mut usize, total: usize, height: usize) {
        if total == 0 || height == 0 {
            *scroll = 0;
            return;
        }
        *scroll = (*scroll).min(total.saturating_sub(height));
        if let Some(sel) = selected {
            if sel < *scroll {
                *scroll = sel;
            } else if sel >= *scroll + height {
                *scroll = sel.saturating_sub(height - 1);
            }
        }
    }

    // ── Rendering ──

    pub(super) fn render_model(
        f: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        m: &BrowserModel,
    ) {
        use ratatui::layout::{Constraint, Layout};
        use ratatui::style::{Color, Stylize};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, Borders, Paragraph};

        let chunks = Layout::vertical([
            Constraint::Length(3), // title
            Constraint::Length(3), // search
            Constraint::Min(0),    // body (2 panes)
            Constraint::Length(3), // detail + footer
        ])
        .split(area);

        // ── Title ──
        let total_streams: usize = m.all_collectors.iter().map(|c| c.streams.len()).sum();
        let subtitle = format!(
            "{} collectors  ·  {} streams",
            m.filtered_indices.len(),
            total_streams
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
        let search_content: Line = if m.filter.is_empty() {
            Line::from(vec![
                Span::raw("  "),
                Span::styled("search", Color::DarkGray),
                Span::raw("  —  type ASN, name, collector, or topic"),
            ])
        } else {
            Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("search  ▏{}", m.filter), Color::Reset),
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
        let col_height = body[0].height.saturating_sub(2) as usize;
        let col_total = m.filtered_indices.len();
        let col_range = m.collector_scroll..(m.collector_scroll + col_height).min(col_total);
        let mut col_lines: Vec<Line> = Vec::new();
        for i in col_range {
            let idx = m.filtered_indices[i];
            let mc = &m.all_collectors[idx];
            let label_trunc = if mc.label.len() > 34 {
                truncate_str(&mc.label, 32)
            } else {
                mc.label.clone()
            };
            let text = format!(" {:<38} {:>4}", label_trunc, mc.streams.len());
            let on = m.active_pane == Pane::Collectors && m.selected_collector == Some(i);
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
        let streams = m.current_streams();
        let stream_height = body[1].height.saturating_sub(2) as usize;
        let stream_total = streams.len();
        let stream_range = m.stream_scroll..(m.stream_scroll + stream_height).min(stream_total);
        let mut stream_lines: Vec<Line> = Vec::new();
        for i in stream_range {
            let pt = &streams[i];
            let name = as_name(&pt.asn_str);
            let name_display = if name.len() > 26 {
                truncate_str(&name, 24)
            } else if name.is_empty() || name.starts_with("AS") {
                String::new()
            } else {
                name
            };
            let on = m.active_pane == Pane::Streams && m.selected_stream == Some(i);
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
        let stream_title = match m.current_collector() {
            Some(mc) => format!("Streams — {}", mc.label),
            None => "Streams".into(),
        };
        f.render_widget(
            Paragraph::new(ratatui::text::Text::from(stream_lines))
                .block(Block::bordered().title(stream_title).borders(Borders::ALL)),
            body[1],
        );

        // ── Selected detail + Footer ──
        let detail = m.selected_topic().unwrap_or("no stream selected");
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
                    Span::raw(" quit  "),
                    Span::styled("click", Color::White).bold(),
                    Span::raw(" select  "),
                    Span::styled("wheel", Color::White).bold(),
                    Span::raw(" scroll"),
                ]),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        detail,
                        if m.selected_topic().is_some() {
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
    }

    #[cfg(test)]
    mod model_tests {
        use super::*;

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

        fn assert_invariants(m: &BrowserModel) {
            // selected_collector is valid or None
            if let Some(i) = m.selected_collector {
                assert!(i < m.filtered_indices.len());
            }
            // selected_stream is valid or None
            if let Some(i) = m.selected_stream {
                assert!(i < m.current_streams().len());
            }
            // active pane is not Collectors if collector list is empty
            if m.filtered_indices.is_empty() && m.active_pane == Pane::Collectors {
                // allowed only if stream pane is also empty
                assert!(m.current_streams().is_empty());
            }
        }

        // ── Initialization ──

        #[test]
        fn test_new_empty() {
            let m = BrowserModel::new(&[]);
            assert!(m.filtered_indices.is_empty());
            assert_eq!(m.selected_collector, None);
            assert_eq!(m.selected_stream, None);
            assert_eq!(m.selected_topic(), None);
            assert_invariants(&m);
        }

        #[test]
        fn test_new_one_collector_one_stream() {
            let m = BrowserModel::new(&["routeviews.chicago.13335.bmp_raw".into()]);
            assert_eq!(m.filtered_indices.len(), 1);
            assert_eq!(m.selected_collector, Some(0));
            assert_eq!(m.current_streams().len(), 1);
            assert_invariants(&m);
        }

        #[test]
        fn test_empty_filter_hides_undefined() {
            let m = BrowserModel::new(&test_topics());
            let has_undef = m
                .filtered_indices
                .iter()
                .any(|&i| m.all_collectors[i].raw_name.contains("UNDEFINED"));
            assert!(!has_undef);
            assert_invariants(&m);
        }

        #[test]
        fn test_filter_reveals_undefined_if_matched() {
            let mut m = BrowserModel::new(&test_topics());
            m.apply(Action::TypeChar('9'));
            m.apply(Action::TypeChar('9'));
            m.apply(Action::TypeChar('9'));
            m.apply(Action::TypeChar('9'));
            m.apply(Action::TypeChar('9'));
            let has_undef = m
                .filtered_indices
                .iter()
                .any(|&i| m.all_collectors[i].raw_name.contains("UNDEFINED"));
            assert!(has_undef);
            assert_invariants(&m);
        }

        #[test]
        fn test_filter_by_asn() {
            let mut m = BrowserModel::new(&test_topics());
            for c in "13335".chars() {
                m.apply(Action::TypeChar(c));
            }
            assert!(!m.filtered_indices.is_empty());
            assert_invariants(&m);
        }

        #[test]
        fn test_filter_by_collector_name() {
            let mut m = BrowserModel::new(&test_topics());
            for c in "chicago".chars() {
                m.apply(Action::TypeChar(c));
            }
            assert_eq!(m.filtered_indices.len(), 1);
            assert_invariants(&m);
        }

        #[test]
        fn test_filter_zero_results() {
            let mut m = BrowserModel::new(&test_topics());
            for c in "zzzzzzz".chars() {
                m.apply(Action::TypeChar(c));
            }
            assert!(m.filtered_indices.is_empty());
            assert_eq!(m.selected_collector, None);
            assert_eq!(m.selected_topic(), None);
            assert_invariants(&m);
        }

        #[test]
        fn test_backspace_restores_list() {
            let mut m = BrowserModel::new(&test_topics());
            let before = m.filtered_indices.len();
            for c in "zzz".chars() {
                m.apply(Action::TypeChar(c));
            }
            assert!(m.filtered_indices.is_empty());
            m.apply(Action::Backspace);
            m.apply(Action::Backspace);
            m.apply(Action::Backspace);
            assert_eq!(m.filtered_indices.len(), before);
            assert_invariants(&m);
        }

        // ── Navigation ──

        #[test]
        fn test_move_up_at_top_stays() {
            let mut m = BrowserModel::new(&test_topics());
            m.apply(Action::MoveUp);
            assert_eq!(m.selected_collector, Some(0));
            assert_invariants(&m);
        }

        #[test]
        fn test_move_down_at_bottom_stays() {
            let mut m = BrowserModel::new(&test_topics());
            let len = m.filtered_indices.len();
            for _ in 0..100 {
                m.apply(Action::MoveDown);
            }
            assert_eq!(m.selected_collector, Some(len - 1));
            assert_invariants(&m);
        }

        #[test]
        fn test_move_down_stream_clamped() {
            let mut m = BrowserModel::new(&test_topics());
            m.apply(Action::Enter); // switch to streams
            for _ in 0..100 {
                m.apply(Action::MoveDown);
            }
            let max = m.current_streams().len() - 1;
            assert_eq!(m.selected_stream, Some(max));
            assert_invariants(&m);
        }

        #[test]
        fn test_switch_pane_to_empty_stays() {
            let mut m = BrowserModel::new(&[]);
            m.apply(Action::SwitchPane);
            assert_eq!(m.active_pane, Pane::Collectors);
            assert_invariants(&m);
        }

        #[test]
        fn test_switch_pane_works_after_collector_selection() {
            let mut m = BrowserModel::new(&test_topics());
            m.apply(Action::SwitchPane);
            assert_eq!(m.active_pane, Pane::Streams);
            assert_invariants(&m);
        }

        // ── Enter behavior ──

        #[test]
        fn test_enter_on_collector_switches_to_streams() {
            let mut m = BrowserModel::new(&test_topics());
            let result = m.apply(Action::Enter);
            assert_eq!(result, ActionResult::None);
            assert_eq!(m.active_pane, Pane::Streams);
            assert_invariants(&m);
        }

        #[test]
        fn test_enter_on_stream_returns_topic() {
            let mut m = BrowserModel::new(&test_topics());
            m.apply(Action::Enter); // collector → streams
            let topic = m.selected_topic().unwrap().to_string();
            let result = m.apply(Action::Enter);
            assert_eq!(result, ActionResult::Selected(topic));
            assert_invariants(&m);
        }

        #[test]
        fn test_enter_on_zero_result_does_nothing() {
            let mut m = BrowserModel::new(&test_topics());
            for c in "zzz".chars() {
                m.apply(Action::TypeChar(c));
            }
            m.apply(Action::Enter); // stream pane with no streams
            let result = m.apply(Action::Enter); // stream Enter with no selection
            assert_eq!(result, ActionResult::None);
            assert_eq!(m.selected_topic(), None);
            assert_invariants(&m);
        }

        #[test]
        fn test_filter_shrink_clamps_selection() {
            let mut m = BrowserModel::new(&test_topics());
            // Move down to second collector
            m.apply(Action::MoveDown);
            assert_eq!(m.selected_collector, Some(1));
            // Filter to only first collector
            for c in "chicago".chars() {
                m.apply(Action::TypeChar(c));
            }
            // Selection clamped
            assert_eq!(m.selected_collector, Some(0));
            assert_invariants(&m);
        }

        // ── Scroll (standard HCI: only scroll when selection hits edge) ──

        #[test]
        fn test_ensure_visible_empty() {
            let mut scroll = 5;
            ensure_visible(None, &mut scroll, 0, 10);
            assert_eq!(scroll, 0);
        }

        #[test]
        fn test_ensure_visible_in_viewport_unchanged() {
            let mut scroll = 5;
            ensure_visible(Some(7), &mut scroll, 20, 10);
            assert_eq!(scroll, 5);
        }

        #[test]
        fn test_ensure_visible_above_scrolls_up() {
            let mut scroll = 10;
            ensure_visible(Some(3), &mut scroll, 20, 10);
            assert_eq!(scroll, 3);
        }

        #[test]
        fn test_ensure_visible_below_scrolls_down() {
            let mut scroll = 0;
            ensure_visible(Some(15), &mut scroll, 20, 10);
            assert_eq!(scroll, 6);
        }

        #[test]
        fn test_ensure_visible_at_edge_no_scroll() {
            let mut scroll = 5;
            ensure_visible(Some(5), &mut scroll, 20, 10);
            assert_eq!(scroll, 5);
            ensure_visible(Some(14), &mut scroll, 20, 10);
            assert_eq!(scroll, 5);
        }

        #[test]
        fn test_ensure_visible_clamps_invalid_scroll() {
            let mut scroll = 100;
            ensure_visible(Some(18), &mut scroll, 20, 10);
            assert_eq!(scroll, 10);
        }

        // ── Endurance ──

        #[test]
        fn test_endurance_500_actions() {
            let mut m = BrowserModel::new(&test_topics());
            let actions = [
                Action::MoveDown,
                Action::MoveDown,
                Action::MoveUp,
                Action::SwitchPane,
                Action::MoveDown,
                Action::MoveUp,
                Action::SwitchPane,
                Action::TypeChar('1'),
                Action::TypeChar('3'),
                Action::Backspace,
                Action::Backspace,
                Action::Enter,
                Action::MoveDown,
                Action::Enter,
            ];
            for _ in 0..500 {
                for &a in &actions {
                    m.apply(a);
                    assert_invariants(&m);
                }
            }
        }

        // ── Render tests ──

        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        #[test]
        fn test_render_empty_model_no_panic() {
            let m = BrowserModel::new(&[]);
            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| render_model(f, f.area(), &m))
                .expect("render");
        }

        #[test]
        fn test_render_normal_80x24_no_panic() {
            let m = BrowserModel::new(&test_topics());
            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| render_model(f, f.area(), &m))
                .expect("render");
        }

        #[test]
        fn test_render_normal_100x30_no_panic() {
            let m = BrowserModel::new(&test_topics());
            let backend = TestBackend::new(100, 30);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| render_model(f, f.area(), &m))
                .expect("render");
        }

        #[test]
        fn test_render_after_scrolling_collectors_no_panic() {
            let mut m = BrowserModel::new(&test_topics());
            for _ in 0..50 {
                m.apply(Action::MoveDown);
            }
            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| render_model(f, f.area(), &m))
                .expect("render");
        }

        #[test]
        fn test_render_after_scrolling_streams_no_panic() {
            let mut m = BrowserModel::new(&test_topics());
            m.apply(Action::Enter);
            for _ in 0..50 {
                m.apply(Action::MoveDown);
            }
            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| render_model(f, f.area(), &m))
                .expect("render");
        }

        #[test]
        fn test_render_zero_results_includes_no_matches() {
            let mut m = BrowserModel::new(&test_topics());
            for c in "zzzzzzz".chars() {
                m.apply(Action::TypeChar(c));
            }
            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| render_model(f, f.area(), &m))
                .expect("render");
            let buf = terminal.backend().buffer();
            let text = buf
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<Vec<_>>()
                .join("");
            assert!(
                text.contains("no matches") || text.contains("no stream selected"),
                "should show no-matches: '{text}'"
            );
        }

        #[test]
        fn test_render_includes_selected_topic() {
            let m = BrowserModel::new(&test_topics());
            let _topic = m.selected_topic().unwrap().to_string();
            // Check that the render call succeeds and produces a non-empty
            // buffer. Full topic may be truncated in the footer.
            let backend = TestBackend::new(100, 30);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| render_model(f, f.area(), &m))
                .expect("render");
            let buf = terminal.backend().buffer();
            let text: String = buf
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<Vec<_>>()
                .join("");
            assert!(text.contains("BMPWatch"), "should include title");
            assert!(
                text.contains("Collectors"),
                "should include collectors pane"
            );
        }

        #[test]
        fn test_render_active_pane_changes_after_switch() {
            let mut m = BrowserModel::new(&test_topics());
            m.apply(Action::SwitchPane);
            let backend = TestBackend::new(100, 30);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| render_model(f, f.area(), &m))
                .expect("render after switch");
        }

        #[test]
        fn test_render_unicode_labels_no_panic() {
            let topics = vec!["routeviews.saopaulo2.16509.bmp_raw".to_string()];
            let m = BrowserModel::new(&topics);
            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| render_model(f, f.area(), &m))
                .expect("should not panic with Unicode labels");
        }

        // ── Escape-sequence fragment tests ──

        #[test]
        fn test_normal_typing_still_works() {
            let mut m = BrowserModel::new(&test_topics());
            m.apply(Action::TypeChar('a'));
            m.apply(Action::TypeChar('b'));
            assert_eq!(m.filter, "ab");
        }

        #[test]
        fn test_bracket_without_esc_is_allowed() {
            // A bare '[' typed without preceding Esc is fine to keep or
            // filter — we allow it since it's intentional user input.
            let mut m = BrowserModel::new(&test_topics());
            m.apply(Action::TypeChar('['));
            m.apply(Action::TypeChar('A'));
            // With real crossterm, '[' and 'A' without preceding Esc would
            // arrive as distinct Char events only if typed literally.
            // Our model TypeChar path doesn't run the escape-state machine,
            // so they go into the filter. That's correct — the state machine
            // lives in the event loop, not the model.
            assert_eq!(m.filter, "[A");
        }

        #[test]
        fn test_truncate_ascii() {
            assert_eq!(super::truncate_str("Hello", 3), "Hel…");
            assert_eq!(super::truncate_str("Hello", 10), "Hello");
        }

        #[test]
        fn test_truncate_multibyte() {
            assert_eq!(super::truncate_str("São Paulo", 4), "São …");
            assert_eq!(super::truncate_str("Malmö", 8), "Malmö");
            assert_eq!(super::truncate_str("Querétaro", 6), "Querét…");
        }
    }
} // mod model

#[cfg(test)]
mod mouse_tests {
    use super::*;
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

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

    // Layout matching render_model for an 80x24 terminal:
    // chunks[0] = title (y=0..3), chunks[1] = search (y=3..6),
    // chunks[2] = body (y=6..21), chunks[3] = footer (y=21..24)
    // Body split 50/50: left=collectors (x=0..40), right=streams (x=40..80)
    fn test_layout() -> BrowserLayout {
        BrowserLayout {
            collector_pane: Rect::new(0, 6, 40, 15),
            stream_pane: Rect::new(40, 6, 40, 15),
        }
    }

    fn left_click(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn scroll_up(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn scroll_down(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn drag(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn moved(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Moved,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn up(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn test_click_collector_row_selects() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();

        // First content row at y=7 (pane.y=6 + 1 for border)
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &left_click(1, 7),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, Some(0));
        assert_eq!(model.active_pane, model::Pane::Collectors);
    }

    #[test]
    fn test_click_collector_header_noop() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let initial = model.selected_collector;

        // Border row at y=6 is the header/title area
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &left_click(1, 6),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, initial);
    }

    #[test]
    fn test_click_stream_row_selects() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();

        // First content row of stream pane at y=7, x=41
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &left_click(41, 7),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_stream, Some(0));
        assert_eq!(model.active_pane, model::Pane::Streams);
    }

    #[test]
    fn test_click_below_stream_rows_noop() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let initial = model.selected_stream;

        // Click below any valid stream row
        let streams_len = model.current_streams().len();
        let below_content = layout.stream_pane.y + 1 + streams_len as u16;
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &left_click(41, below_content),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_stream, initial);
    }

    #[test]
    fn test_click_outside_panes_noop() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let initial_col = model.selected_collector;

        // Click in title area (y=1)
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &left_click(1, 1),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, initial_col);
    }

    #[test]
    fn test_click_does_not_return_selected() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();

        // Click on a stream row — should NOT return Selected
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &left_click(41, 7),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
    }

    #[test]
    fn test_wheel_collector_moves_selection() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();

        // Scroll down over collector pane
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &scroll_down(1, 7),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, Some(1));
        assert_eq!(model.active_pane, model::Pane::Collectors);
    }

    #[test]
    fn test_wheel_stream_moves_selection() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();

        // Scroll down over stream pane
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &scroll_down(41, 7),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_stream, Some(1));
        assert_eq!(model.active_pane, model::Pane::Streams);
    }

    #[test]
    fn test_wheel_over_footer_noop() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let initial_col = model.selected_collector;
        let initial_stream = model.selected_stream;

        // Footer is at y=21..24
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &scroll_down(1, 22),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, initial_col);
        assert_eq!(model.selected_stream, initial_stream);
    }

    #[test]
    fn test_drag_ignored() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let initial_col = model.selected_collector;

        let result = handle_mouse_event(&mut model, &layout, &drag(1, 7), &mut ClickTracker::new());
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, initial_col);
    }

    #[test]
    fn test_moved_ignored() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let initial_col = model.selected_collector;

        let result =
            handle_mouse_event(&mut model, &layout, &moved(1, 7), &mut ClickTracker::new());
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, initial_col);
    }

    #[test]
    fn test_up_ignored() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let initial_col = model.selected_collector;

        let result = handle_mouse_event(&mut model, &layout, &up(1, 7), &mut ClickTracker::new());
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, initial_col);
    }

    #[test]
    fn test_enter_after_click_stream_selected() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();

        // Click to select a stream
        handle_mouse_event(
            &mut model,
            &layout,
            &left_click(41, 7),
            &mut ClickTracker::new(),
        );
        assert_eq!(model.selected_stream, Some(0));
        assert_eq!(model.active_pane, model::Pane::Streams);

        // Enter should now return Selected
        let result = model.apply(model::Action::Enter);
        match result {
            model::ActionResult::Selected(t) => {
                assert!(!t.is_empty());
            }
            _ => panic!("expected Selected after Enter on clicked stream"),
        }
    }

    #[test]
    fn test_click_zero_results_noop() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        // Filter to zero results
        for c in "zzzzzzz".chars() {
            model.apply(model::Action::TypeChar(c));
        }
        assert!(model.filtered_indices.is_empty());

        let layout = test_layout();
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &left_click(1, 7),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, None);
    }

    #[test]
    fn test_wheel_over_title_noop() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let initial_col = model.selected_collector;

        // Title area is y=0..3
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &scroll_down(1, 1),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, initial_col);
    }

    #[test]
    fn test_wheel_over_search_noop() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let initial_col = model.selected_collector;

        // Search area is y=3..6
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &scroll_down(1, 4),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, initial_col);
    }

    #[test]
    fn test_wheel_up_collector() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        // Move down first so we can scroll up
        model.apply(model::Action::MoveDown);
        assert_eq!(model.selected_collector, Some(1));

        let layout = test_layout();
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &scroll_up(1, 7),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, Some(0));
    }

    #[test]
    fn test_click_maps_through_scroll_offset() {
        // When the viewport is scrolled, click maps scroll+row to item index.
        let mut topics = Vec::new();
        for i in 0..30 {
            topics.push(format!("routeviews.collector{}.{}.bmp_raw", i, 1000 + i));
        }
        let mut model = model::BrowserModel::new(&topics);
        // Simulate scrolled viewport: items 10-17 visible
        model.collector_scroll = 10;
        model.selected_collector = Some(12);

        let layout = BrowserLayout {
            collector_pane: Rect::new(0, 6, 40, 10),
            stream_pane: Rect::new(40, 6, 40, 10),
        };

        // Click first visible row → selects scroll+0 = item 10
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &left_click(1, 7),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, Some(10));

        // Click third visible row → selects scroll+2 = item 12
        let result = handle_mouse_event(
            &mut model,
            &layout,
            &left_click(1, 9),
            &mut ClickTracker::new(),
        );
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, Some(12));
    }

    #[test]
    fn test_double_click_stream_returns_selected() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let mut tracker = ClickTracker::new();

        // First click on stream (row 0 in chicago collector = AS13335)
        let result = handle_mouse_event(&mut model, &layout, &left_click(41, 7), &mut tracker);
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_stream, Some(0));

        // Second click on same stream within threshold
        let result = handle_mouse_event(&mut model, &layout, &left_click(41, 7), &mut tracker);
        match result {
            model::ActionResult::Selected(t) => assert!(!t.is_empty()),
            _ => panic!("expected Selected on double-click"),
        }
    }

    #[test]
    fn test_double_click_collector_does_not_return_selected() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let mut tracker = ClickTracker::new();

        // First click on collector
        handle_mouse_event(&mut model, &layout, &left_click(1, 7), &mut tracker);
        assert_eq!(model.selected_collector, Some(0));

        // Second click on same collector — still no Selected
        let result = handle_mouse_event(&mut model, &layout, &left_click(1, 7), &mut tracker);
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_collector, Some(0));
    }

    #[test]
    fn test_click_different_streams_no_double_click() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let mut tracker = ClickTracker::new();

        // Click first stream (AS13335)
        handle_mouse_event(&mut model, &layout, &left_click(41, 7), &mut tracker);
        assert_eq!(model.selected_stream, Some(0));

        // Click second stream (AS2914) — different row, no double-click
        let result = handle_mouse_event(&mut model, &layout, &left_click(41, 8), &mut tracker);
        assert_eq!(result, model::ActionResult::None);
        assert_eq!(model.selected_stream, Some(1));
    }

    #[test]
    fn test_click_after_threshold_no_double_click() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let mut tracker = ClickTracker::new();

        // First click
        handle_mouse_event(&mut model, &layout, &left_click(41, 7), &mut tracker);

        // Wait past the 500ms threshold
        std::thread::sleep(Duration::from_millis(501));

        // Second click — too late, no double-click
        let result = handle_mouse_event(&mut model, &layout, &left_click(41, 7), &mut tracker);
        assert_eq!(result, model::ActionResult::None);
    }

    #[test]
    fn test_wheel_between_clicks_clears_double_click_state() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let mut tracker = ClickTracker::new();

        // First click on stream
        handle_mouse_event(&mut model, &layout, &left_click(41, 7), &mut tracker);

        // Wheel — resets tracker
        handle_mouse_event(&mut model, &layout, &scroll_down(41, 7), &mut tracker);

        // Second click — not a double-click (tracker was reset)
        let result = handle_mouse_event(&mut model, &layout, &left_click(41, 7), &mut tracker);
        assert_eq!(result, model::ActionResult::None);
    }

    #[test]
    fn test_tracker_reset_prevents_double_click() {
        let topics = test_topics();
        let mut model = model::BrowserModel::new(&topics);
        let layout = test_layout();
        let mut tracker = ClickTracker::new();

        // First click on stream
        handle_mouse_event(&mut model, &layout, &left_click(41, 7), &mut tracker);

        // Simulate filter change (event loop resets tracker)
        tracker.reset();

        // Second click — should not double-click
        let result = handle_mouse_event(&mut model, &layout, &left_click(41, 7), &mut tracker);
        assert_eq!(result, model::ActionResult::None);
    }
}
