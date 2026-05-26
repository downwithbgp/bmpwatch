use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use rdkafka::consumer::Consumer;
use rdkafka::Message;

use crate::kafka;
use crate::lint;
use crate::obmp_reader::parse_record_payload;
use crate::raw_bmp::BMP_EXPECTED_VERSION;
use crate::rolling::RollingSummary;
use crate::rpki::RPKICache;
use crate::state::{Finding, PeerKey};
use bytes::Bytes;

const THROUGHPUT_HISTORY: usize = 60;

/// A single line in the scrolling message ticker at the bottom of the screen.
struct MessageLine {
    msg_type: u8,
    asn: Option<u32>,
    ip_short: Option<String>,
    ts_sec: u32,
    prefixes: Option<PrefixChange>,
}

pub(crate) struct Dashboard {
    rolling: RollingSummary,
    topic: String,
    total_messages: u64,
    throughput_history: Vec<u64>,
    current_tick_count: u64,
    last_tick: Instant,
    exit: bool,
    browse: bool,
    paused: bool,
    show_communities: bool,
    peer_msg_counts: HashMap<PeerKey, u64>,
    peer_warnings: HashMap<PeerKey, u64>,
    metadata: Option<crate::obmp_reader::OpenBmpMetadata>,
    message_log: Vec<MessageLine>,
    max_log_lines: usize,
    churn_counts: HashMap<String, u64>,
    prefix_origins: HashMap<String, u32>,
    prefix_last_path: HashMap<String, Vec<u32>>,
    as_adjacency: HashMap<(u32, u32), u64>,
    as_frequency: HashMap<u32, u64>,
    rpki: Option<RPKICache>,
    keepalive_count: u64,
    session_start: Instant,
}

impl Dashboard {
    fn new(topic: &str, window_messages: usize) -> Self {
        Dashboard {
            rolling: RollingSummary::new(window_messages),
            topic: topic.to_string(),
            total_messages: 0,
            throughput_history: vec![0; THROUGHPUT_HISTORY],
            current_tick_count: 0,
            last_tick: Instant::now(),
            exit: false,
            browse: false,
            paused: false,
            show_communities: false,
            peer_msg_counts: HashMap::new(),
            peer_warnings: HashMap::new(),
            metadata: None,
            message_log: Vec::new(),
            max_log_lines: 500,
            churn_counts: HashMap::new(),
            prefix_origins: HashMap::new(),
            prefix_last_path: HashMap::new(),
            as_adjacency: HashMap::new(),
            as_frequency: HashMap::new(),
            rpki: None,
            keepalive_count: 0,
            session_start: Instant::now(),
        }
    }

    fn process_message(&mut self, payload: &[u8]) {
        let record = parse_record_payload(payload, 0, self.total_messages);
        self.total_messages += 1;
        self.current_tick_count += 1;

        match record.frame {
            Ok(frame) => {
                let peer_key = frame
                    .per_peer_header
                    .as_ref()
                    .map(crate::doctor::peer_key_from_pph);

                let mut findings: Vec<Finding> = Vec::new();
                if frame.version != BMP_EXPECTED_VERSION {
                    findings.push(lint::finding_invalid_version(frame.offset, frame.version));
                }
                if frame.msg_type.is_none() {
                    findings.push(lint::finding_unknown_type(frame.offset, frame.msg_type_raw));
                }

                if let Some(ref pk) = peer_key {
                    *self.peer_msg_counts.entry(pk.clone()).or_insert(0) += 1;
                    if !findings.is_empty() {
                        *self.peer_warnings.entry(pk.clone()).or_insert(0) += findings.len() as u64;
                    }
                }

                let mut prefixes = extract_prefixes(&frame.full_data);

                // Keepalive: BGP UPDATE with no prefix changes — count, skip log.
                let is_keepalive = matches!(
                    &prefixes,
                    Some(pc) if pc.announced.is_empty() && pc.withdrawn.is_empty()
                );
                if is_keepalive {
                    self.keepalive_count += 1;
                } else {
                    // RPKI validation
                    if let Some(ref mut pc) = prefixes {
                        let mut worst = None;
                        for (p, origin) in &pc.announced {
                            if let Some(ref mut rpki) = self.rpki {
                                let (status, detail) = rpki.validate(p, *origin);
                                worst = Some(match worst {
                                    None => status,
                                    Some(s @ crate::rpki::Status::Invalid)
                                    | Some(s @ crate::rpki::Status::InvalidWrongAsn)
                                    | Some(s @ crate::rpki::Status::InvalidTooLong) => s,
                                    Some(crate::rpki::Status::NotFound) => {
                                        if matches!(
                                            status,
                                            crate::rpki::Status::Invalid
                                                | crate::rpki::Status::InvalidWrongAsn
                                                | crate::rpki::Status::InvalidTooLong
                                        ) {
                                            status
                                        } else {
                                            crate::rpki::Status::NotFound
                                        }
                                    }
                                    Some(s) => s,
                                });
                                // Store detail for the first invalid prefix
                                if pc.rpki_detail.is_none()
                                    && matches!(
                                        status,
                                        crate::rpki::Status::InvalidWrongAsn
                                            | crate::rpki::Status::InvalidTooLong
                                    )
                                {
                                    pc.rpki_detail = Some(detail);
                                }
                            }
                        }
                        pc.rpki = worst;
                    }
                    // Track churn, origins, and AS relationships
                    if let Some(ref pc) = prefixes {
                        for (p, origin) in &pc.announced {
                            *self.churn_counts.entry(p.clone()).or_insert(0) += 1;
                            if *origin > 0 {
                                self.prefix_origins.insert(p.clone(), *origin);
                            }
                            self.prefix_last_path.insert(p.clone(), pc.as_path.clone());
                        }
                        for (p, origin) in &pc.withdrawn {
                            *self.churn_counts.entry(p.clone()).or_insert(0) += 1;
                            if *origin > 0 {
                                self.prefix_origins.insert(p.clone(), *origin);
                            }
                        }
                        for asn in &pc.as_path {
                            *self.as_frequency.entry(*asn).or_insert(0) += 1;
                        }
                        for pair in pc.as_path.windows(2) {
                            let key = (pair[0], pair[1]);
                            *self.as_adjacency.entry(key).or_insert(0) += 1;
                        }
                    }
                    let ts_sec = frame
                        .per_peer_header
                        .as_ref()
                        .map(|pph| pph.timestamp_seconds)
                        .unwrap_or(0);
                    let ip_short = peer_key
                        .as_ref()
                        .and_then(|pk| pk.peer_ip.as_ref())
                        .map(|ip| {
                            if ip.len() > 18 {
                                format!("{}..", &ip[..16])
                            } else {
                                ip.clone()
                            }
                        });
                    self.message_log.push(MessageLine {
                        msg_type: frame.msg_type_raw,
                        asn: peer_key.as_ref().and_then(|pk| pk.peer_asn),
                        ip_short,
                        ts_sec,
                        prefixes,
                    });
                    // Keep log bounded
                    while self.message_log.len() > self.max_log_lines {
                        self.message_log.remove(0);
                    }
                } // end else (non-keepalive)

                // Capture OpenBMP metadata for the header
                if self.metadata.is_none() {
                    if let Some(ref meta) = record.metadata {
                        if meta.any() {
                            self.metadata = Some(meta.clone());
                        }
                    }
                }
                self.rolling
                    .set_metadata(self.metadata.clone().unwrap_or_default());

                self.rolling
                    .push(frame.msg_type_raw, false, findings, peer_key);
            }
            Err(_) => {
                self.rolling.push(0, true, vec![], None);
            }
        }
    }

    fn tick(&mut self) {
        let elapsed = self.last_tick.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let rate = self.current_tick_count;
            self.current_tick_count = 0;
            self.last_tick = Instant::now();

            self.throughput_history.rotate_left(1);
            let len = self.throughput_history.len();
            self.throughput_history[len - 1] = rate;
        }
    }

    fn current_rate(&self) -> u64 {
        self.throughput_history.last().copied().unwrap_or(0)
    }
}

fn handle_key(key: event::KeyEvent, dash: &mut Dashboard) {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            dash.exit = true;
        }
        KeyCode::Char('q') | KeyCode::Esc => dash.exit = true,
        KeyCode::Char('b') => {
            dash.browse = true;
            dash.exit = true;
        }
        KeyCode::Char('p') => dash.paused = !dash.paused,
        KeyCode::Char('c') => dash.show_communities = !dash.show_communities,
        _ => {}
    }
}

pub(crate) fn run_dashboard(
    broker: &str,
    topic: Option<&str>,
    collector: Option<&str>,
    asn: Option<&str>,
    window_messages: usize,
    _mock: bool,
    _mock_active: bool,
) -> Result<()> {
    // Phase 1: Resolve the topic (may need TUI for the browser)
    let mut terminal: DefaultTerminal = ratatui::init();

    let mut chosen = if let Some(exact) = topic {
        exact.to_string()
    } else {
        let all = kafka::fetch_topics(broker, "^routeviews.*\\.bmp_raw$")?;
        let mut filtered = kafka::apply_filters(all, collector, asn);
        filtered.sort_by(|a, b| {
            let a_undef = a.contains("UNDEFINED_ROUTER_GROUP");
            let b_undef = b.contains("UNDEFINED_ROUTER_GROUP");
            a_undef.cmp(&b_undef).then_with(|| a.cmp(b))
        });

        // Filter to active peerings only (peering-status.html).
        // This hides Kafka-advertised streams for dead or inactive peerings.
        let active = crate::peering::load_active_peering_set();
        filtered = crate::peering::filter_active_topics(&filtered, &active);

        if filtered.is_empty() {
            let msg = if collector.is_some() || asn.is_some() {
                "No topics matched filters. Try broader --collector/--asn, or run without filters."
            } else {
                "No RouteViews topics found on this broker."
            };
            terminal.draw(|f| {
                let area = f.area();
                let text = Paragraph::new(msg)
                    .block(Block::bordered().title("Error").borders(Borders::ALL));
                f.render_widget(text, area);
            })?;
            std::thread::sleep(Duration::from_secs(3));
            ratatui::restore();
            anyhow::bail!("{msg}");
        }

        if filtered.len() == 1 {
            filtered.into_iter().next().unwrap()
        } else {
            match topic_browser(&mut terminal, &filtered)? {
                Some(t) => t,
                None => {
                    ratatui::restore();
                    return Ok(());
                }
            }
        }
    };

    // Stay in the alternate screen — no ratatui::restore() between
    // browser and dashboard. The terminal remains active throughout.

    loop {
        // Setup consumer and dashboard
        let consumer = kafka::create_consumer(broker, "bmpwatch-dashboard", true)?;
        consumer
            .subscribe(&[&chosen])
            .map_err(|e| anyhow::anyhow!("Failed to subscribe to topic '{chosen}': {e}"))?;

        let mut dash = Dashboard::new(&chosen, window_messages);

        // Load caches and RPKI inside the TUI (no shell output)
        terminal.draw(|f| render_loading(f, f.area(), &chosen, broker))?;
        load_as_name_cache();
        if let Ok(mut cache) = RPKICache::load_or_download("rtr.rpki.cloudflare.com", 8282) {
            // Re-validate any TUI-primed messages already in the log.
            for msg in &mut dash.message_log {
                if let Some(ref mut pc) = msg.prefixes {
                    let mut worst = None;
                    for (p, origin) in &pc.announced {
                        let (status, detail) = cache.validate(p, *origin);
                        worst = Some(match worst {
                            None => status,
                            Some(s @ crate::rpki::Status::Invalid)
                            | Some(s @ crate::rpki::Status::InvalidWrongAsn)
                            | Some(s @ crate::rpki::Status::InvalidTooLong) => s,
                            Some(crate::rpki::Status::NotFound) => {
                                if matches!(
                                    status,
                                    crate::rpki::Status::Invalid
                                        | crate::rpki::Status::InvalidWrongAsn
                                        | crate::rpki::Status::InvalidTooLong
                                ) {
                                    status
                                } else {
                                    crate::rpki::Status::NotFound
                                }
                            }
                            Some(s) => s,
                        });
                        if pc.rpki_detail.is_none()
                            && matches!(
                                status,
                                crate::rpki::Status::InvalidWrongAsn
                                    | crate::rpki::Status::InvalidTooLong
                            )
                        {
                            pc.rpki_detail = Some(detail);
                        }
                    }
                    pc.rpki = worst;
                }
            }
            dash.rpki = Some(cache);
        }

        // Dashboard TUI loop — reuses the same terminal (no re-init).
        let result = run_loop(&mut terminal, &consumer, &mut dash);

        if !dash.browse {
            ratatui::restore();
            println!(
                "topic: {}\ntotal_messages: {}\nmalformed: {}\npeers_observed: {}",
                dash.topic,
                dash.total_messages,
                dash.rolling.malformed_messages(),
                dash.rolling.peers_observed(),
            );
            return result;
        }

        // User pressed 'b' — back to browser without leaving alternate screen.
        let all = kafka::fetch_topics(broker, "^routeviews.*\\.bmp_raw$")?;
        let mut filtered = kafka::apply_filters(all, None, None);
        filtered.sort_by(|a, b| {
            let a_undef = a.contains("UNDEFINED_ROUTER_GROUP");
            let b_undef = b.contains("UNDEFINED_ROUTER_GROUP");
            a_undef.cmp(&b_undef).then_with(|| a.cmp(b))
        });
        let active = crate::peering::load_active_peering_set();
        filtered = crate::peering::filter_active_topics(&filtered, &active);
        match topic_browser(&mut terminal, &filtered)? {
            Some(t) => chosen = t,
            None => {
                ratatui::restore();
                return Ok(());
            }
        }
        // Loop back: setup consumer for new topic, re-enter dashboard.
    }
}
use std::sync::OnceLock;

fn as_name_seed() -> &'static HashMap<u32, String> {
    static SEED: OnceLock<HashMap<u32, String>> = OnceLock::new();
    SEED.get_or_init(|| {
        let data = include_str!("as_names.txt");
        let mut map = HashMap::new();
        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((asn_str, name)) = line.split_once('|') {
                if let Ok(asn) = asn_str.parse::<u32>() {
                    map.insert(asn, name.to_string());
                }
            }
        }
        map
    })
}

/// Strip redundant ASN prefix from Team Cymru names.
pub(crate) fn normalize_cymru_name(asn: u32, raw: &str) -> String {
    let name = raw.trim();
    if name.is_empty() {
        return String::new();
    }
    let prefixes = [
        format!("AS{asn} - "),
        format!("AS{asn} "),
        format!("{asn} - "),
        format!("{asn} "),
    ];
    for prefix in &prefixes {
        if let Some(stripped) = name.strip_prefix(prefix.as_str()) {
            let s = stripped.trim();
            if !s.is_empty() {
                return s.to_string();
            }
            return name.to_string();
        }
    }
    // Strip handle-style prefix: "COGENT-174 - Description"
    if let Some((handle, rest)) = name.split_once(" - ") {
        let is_handle = handle
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-');
        let is_different_asn = handle.starts_with("AS") && !handle.contains(&asn.to_string());
        if is_handle && !is_different_asn {
            let s = rest.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    name.to_string()
}

/// Bundled Team Cymru seed — broad first-run coverage.
fn as_name_seed_cymru() -> &'static HashMap<u32, String> {
    static SEED: std::sync::OnceLock<HashMap<u32, String>> = std::sync::OnceLock::new();
    SEED.get_or_init(|| {
        let data = include_str!("../data/as_names_cymru.txt");
        let mut map = HashMap::new();
        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() < 5 {
                continue;
            }
            if let Ok(asn) = parts[0].parse::<u32>() {
                let name = normalize_cymru_name(asn, parts[4].trim());
                if !name.is_empty() {
                    map.insert(asn, name);
                }
            }
        }
        map
    })
}

/// Bundled RouteViews peer metadata — collector-specific peer names and prefix counts.
struct RouteViewsPeer {
    as_name: String,
    prefixes: u64,
}

/// RouteViews peer metadata index: (collector_key, asn) → peer info.
/// When multiple sessions exist for the same collector+ASN (IPv4 + IPv6),
/// keeps the row with the largest prefix count.
fn routeviews_peers() -> &'static HashMap<(String, u32), RouteViewsPeer> {
    static PEERS: OnceLock<HashMap<(String, u32), RouteViewsPeer>> = OnceLock::new();
    PEERS.get_or_init(|| {
        let data = include_str!("../data/routeviews_peers.tsv");
        let mut map: HashMap<(String, u32), RouteViewsPeer> = HashMap::new();
        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 7 {
                continue;
            }
            let collector_key = parts[0].to_string();
            let asn: u32 = match parts[1].parse() {
                Ok(a) => a,
                Err(_) => continue,
            };
            let prefixes: u64 = match parts[3].parse() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let as_name = parts[6].trim().to_string();
            if as_name.is_empty() {
                continue;
            }
            let key = (collector_key, asn);
            match map.get(&key) {
                Some(existing) if existing.prefixes >= prefixes => {
                    // keep existing row — it has more prefixes
                }
                _ => {
                    map.insert(key, RouteViewsPeer { as_name, prefixes });
                }
            }
        }
        map
    })
}

/// RouteViews peer name fallback by ASN only.
/// When multiple entries exist for the same ASN, prefers the one with the largest prefix count.
fn routeviews_asn_fallback() -> &'static HashMap<u32, String> {
    static FALLBACK: OnceLock<HashMap<u32, String>> = OnceLock::new();
    FALLBACK.get_or_init(|| {
        let mut map: HashMap<u32, (String, u64)> = HashMap::new();
        for ((_col, asn), peer) in routeviews_peers().iter() {
            match map.get(asn) {
                Some((_, existing_pfx)) if *existing_pfx >= peer.prefixes => {
                    // keep existing — larger prefix count
                }
                _ => {
                    map.insert(*asn, (peer.as_name.clone(), peer.prefixes));
                }
            }
        }
        map.into_iter().map(|(k, (name, _))| (k, name)).collect()
    })
}

/// Look up prefix count for a specific collector+ASN peer.
pub(crate) fn routeviews_prefix_count(collector_key: &str, asn: u32) -> Option<u64> {
    routeviews_peers()
        .get(&(collector_key.to_string(), asn))
        .map(|p| p.prefixes)
}

/// Look up RouteViews peer name for a specific collector+ASN (for search).
pub(crate) fn routeviews_peer_name(collector_key: &str, asn: u32) -> Option<&str> {
    routeviews_peers()
        .get(&(collector_key.to_string(), asn))
        .map(|p| p.as_name.as_str())
}

/// Build the active (collector_key, asn) set from the bundled TSV.
/// Excludes zero-prefix entries. Used as ultimate fallback when
/// peering-status.html cannot be fetched and no cache exists.
pub(crate) fn bundled_active_set() -> std::collections::HashSet<(String, u32)> {
    routeviews_peers()
        .iter()
        .filter(|(_, peer)| peer.prefixes > 0)
        .map(|((col, asn), _)| (col.clone(), *asn))
        .collect()
}

/// Format a prefix count for compact display.
pub(crate) fn format_prefix_count(n: u64) -> String {
    if n >= 1_000_000 {
        let m = n as f64 / 1_000_000.0;
        if m >= 10.0 {
            format!("{:.0}M pfx", m)
        } else {
            format!("{:.1}M pfx", m)
        }
    } else if n >= 1_000 {
        format!("{}k pfx", n / 1_000)
    } else {
        format!("{n} pfx")
    }
}

/// Global AS name cache shared between the stream browser and dashboard.
fn global_name_cache() -> &'static Mutex<HashMap<u32, String>> {
    static CACHE: OnceLock<Mutex<HashMap<u32, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve an ASN to a name. Checks global cache → bundled seed → Team Cymru cache →
/// bundled Cymru seed → RouteViews peer fallback.
/// Never blocks. No network I/O. Returns "ASxxxxx" for unknowns.
pub(crate) fn as_name_resolve(asn: u32) -> String {
    {
        let cache = global_name_cache().lock().unwrap();
        if let Some(name) = cache.get(&asn) {
            if !name.is_empty() {
                return name.clone();
            }
            return format!("AS{asn}");
        }
    }
    if let Some(name) = as_name_seed().get(&asn) {
        let name = name.clone();
        global_name_cache()
            .lock()
            .unwrap()
            .insert(asn, name.clone());
        return name;
    }
    // Check Team Cymru cache (no network — loads from disk)
    if let Some(name) = crate::asnames::lookup_cached_name(asn) {
        global_name_cache()
            .lock()
            .unwrap()
            .insert(asn, name.clone());
        return name;
    }
    // Bundled Team Cymru seed — first-run coverage (no network).
    if let Some(name) = as_name_seed_cymru().get(&asn) {
        let name = name.clone();
        global_name_cache()
            .lock()
            .unwrap()
            .insert(asn, name.clone());
        return name;
    }
    // RouteViews peer metadata fallback (by ASN only, no collector context).
    if let Some(name) = routeviews_asn_fallback().get(&asn) {
        let name = name.clone();
        global_name_cache()
            .lock()
            .unwrap()
            .insert(asn, name.clone());
        return name;
    }
    global_name_cache()
        .lock()
        .unwrap()
        .insert(asn, String::new());
    format!("AS{asn}")
}

/// Load bundled seed data into the global AS name cache.
pub(crate) fn load_as_name_cache() {
    let mut cache = global_name_cache().lock().unwrap();
    for (asn, name) in as_name_seed().iter() {
        cache.entry(*asn).or_insert_with(|| name.clone());
    }
}

/// Stream browser name lookup — uses the global cache + seed data.
pub(crate) fn as_name(asn_str: &str) -> String {
    let asn: u32 = match asn_str.parse() {
        Ok(a) => a,
        Err(_) => return String::new(),
    };
    as_name_resolve(asn)
}

fn topic_browser(terminal: &mut DefaultTerminal, topics: &[String]) -> Result<Option<String>> {
    crate::browser::topic_browser(terminal, topics)
}

fn run_loop(
    terminal: &mut DefaultTerminal,
    consumer: &rdkafka::consumer::BaseConsumer,
    dash: &mut Dashboard,
) -> Result<()> {
    while !dash.exit {
        let connected = dash.rolling.total_seen() > 0;
        terminal.draw(|f| render(f, dash, connected))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key(key, dash);
                }
            }
        }

        if !dash.paused {
            let mut got_one = false;
            match consumer.poll(Duration::from_millis(500)) {
                Some(Ok(msg)) => {
                    if let Some(payload) = msg.payload() {
                        dash.process_message(payload);
                        got_one = true;
                    }
                }
                Some(Err(e)) => {
                    // Log Kafka errors to stderr so they're visible after exit
                    eprintln!("Kafka error: {e}");
                }
                None => {}
            }
            if got_one {
                loop {
                    match consumer.poll(Duration::from_millis(0)) {
                        Some(Ok(msg)) => {
                            if let Some(payload) = msg.payload() {
                                dash.process_message(payload);
                            }
                        }
                        Some(Err(e)) => {
                            eprintln!("Kafka error: {e}");
                        }
                        None => break,
                    }
                }
            }
        }

        dash.tick();
    }

    Ok(())
}

fn render_loading(frame: &mut Frame, area: Rect, topic: &str, broker: &str) {
    let lines = vec![
        Line::from(" BMPWatch ").bold().centered(),
        Line::from(""),
        Line::from(format!(" Topic:  {topic}")),
        Line::from(format!(" Broker: {broker}")),
        Line::from(""),
        Line::from(" Loading..."),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().borders(Borders::ALL)),
        area,
    );
}

fn render(frame: &mut Frame, dash: &Dashboard, connected: bool) {
    let area = frame.area();

    let vchunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(3),
    ])
    .split(area);

    render_header(frame, vchunks[0], dash, connected);

    // Body: message log (65%) + route origins (35%)
    let body = Layout::horizontal([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(vchunks[1]);
    render_message_log(frame, body[0], dash);
    render_origins(frame, body[1], dash);

    render_status_bar(frame, vchunks[2], dash);
}

fn render_status_bar(frame: &mut Frame, area: Rect, dash: &Dashboard) {
    let buckets = dash.rolling.findings_buckets();
    let malformed = dash.rolling.malformed_messages();

    let rpki_stats = if let Some(ref rpki) = dash.rpki {
        format!(
            "VAL:{} INV:{} NF:{}  ",
            rpki.valid_count(),
            rpki.invalid_count(),
            rpki.not_found_count(),
        )
    } else {
        String::new()
    };

    let findings = if buckets.parse_errors > 0 || malformed > 0 {
        Span::styled(
            format!(
                " ERR:{} WARN:{} MAL:{}",
                buckets.parse_errors, buckets.stream_order_warnings, malformed
            ),
            Color::Red,
        )
    } else if buckets.stream_order_warnings > 0 {
        Span::styled(
            format!(" WARN:{}", buckets.stream_order_warnings),
            Color::Yellow,
        )
    } else if dash.rolling.total_seen() > 0 {
        Span::styled(" OK", Color::Green)
    } else if dash.session_start.elapsed() >= Duration::from_secs(10) {
        Span::raw(" no messages yet")
    } else {
        Span::raw(" connecting...")
    };

    let rate = format!(" {}/s", dash.current_rate());
    let msgs = if dash.keepalive_count > 0 {
        format!(" msgs:{} KA:{}", dash.total_messages, dash.keepalive_count)
    } else {
        format!(" msgs:{}", dash.total_messages)
    };
    let keys = format!(
        " [q]quit [b]browse [c]comms{} [p]{}",
        if dash.show_communities { ":on" } else { "" },
        if dash.paused { "resume" } else { "pause" },
    );

    let left = format!("{rpki_stats}{msgs}{rate} ");
    let padding_len = (area.width as usize)
        .saturating_sub(left.len() + 20 + keys.len())
        .max(1);

    let bar = Line::from(vec![
        Span::raw(&left),
        findings,
        Span::raw(" ".repeat(padding_len)),
        Span::raw(&keys),
    ]);

    frame.render_widget(
        Paragraph::new(bar).block(Block::bordered().borders(Borders::ALL)),
        area,
    );
}

fn render_header(frame: &mut Frame, area: Rect, dash: &Dashboard, connected: bool) {
    let meta_str = if let Some(ref m) = dash.metadata {
        let collector = m.collector.as_deref().unwrap_or("?");
        let router = m.router.as_deref().unwrap_or("?");
        format!("{collector} / {router}  |  ")
    } else {
        String::new()
    };

    let title = if !connected {
        if dash.session_start.elapsed() >= Duration::from_secs(10) {
            format!(" BMPWatch — {meta_str}No messages yet — stream may be quiet ")
        } else {
            format!(" BMPWatch — {meta_str}Connecting... ")
        }
    } else if dash.paused {
        format!(" BMPWatch — {meta_str}⏸ PAUSED — press p to resume ")
    } else {
        format!(" BMPWatch — {meta_str}{} ", dash.topic)
    };

    let style = if dash.paused || !connected {
        Color::Yellow
    } else {
        Color::Reset
    };
    let header = Paragraph::new(title)
        .bold()
        .style(style)
        .block(Block::bordered().borders(Borders::ALL))
        .centered();
    frame.render_widget(header, area);
}

fn render_message_log(frame: &mut Frame, area: Rect, dash: &Dashboard) {
    // Show as many recent messages as fit
    let max_lines = (area.height.saturating_sub(2)) as usize;
    let total = dash.message_log.len();
    let count = total.min(max_lines);
    let start = total.saturating_sub(count);

    let mut lines: Vec<Line> = Vec::new();
    for msg in &dash.message_log[start..] {
        let type_str = msg_type_label(msg.msg_type);
        let type_color = msg_type_color(msg.msg_type);
        let ts = if msg.ts_sec > 0 {
            let secs_of_day = msg.ts_sec % 86400;
            format!(
                "{:02}:{:02}:{:02}",
                secs_of_day / 3600,
                (secs_of_day % 3600) / 60,
                secs_of_day % 60
            )
        } else {
            String::from("--:--:--")
        };

        match &msg.prefixes {
            Some(pc) => {
                let rpki_str = pc.rpki.map(|s| s.as_str()).unwrap_or("--");
                let rpki_color = match pc.rpki {
                    Some(crate::rpki::Status::Valid) => Color::Green,
                    Some(crate::rpki::Status::Invalid)
                    | Some(crate::rpki::Status::InvalidWrongAsn)
                    | Some(crate::rpki::Status::InvalidTooLong) => Color::Red,
                    Some(crate::rpki::Status::NotFound) => Color::Gray,
                    None => Color::Gray,
                };
                let rpki_hint = pc.rpki_detail.as_ref().map(|d| {
                    let mut parts = Vec::new();
                    if let Some(expected) = d.expected_asn {
                        let name = as_name_resolve(expected);
                        if name.starts_with("AS") {
                            parts.push(format!("should be AS{expected}"));
                        } else {
                            parts.push(format!("should be AS{expected} {name}"));
                        }
                    }
                    if let Some(max_len) = d.max_prefix_len {
                        parts.push(format!("max /{max_len}"));
                    }
                    parts.join(", ")
                });
                let display_path = strip_known_prefix(&pc.as_path, dash);
                let path_str = compact_path(&display_path);
                for (p, _) in &pc.announced {
                    let mut spans = vec![
                        Span::raw(format!("{ts} ")),
                        Span::styled(format!("{} ", rpki_str), rpki_color),
                        Span::styled(format!("{type_str:<8}"), type_color),
                        Span::raw(" +"),
                        Span::styled(p.as_str(), Color::Green),
                    ];
                    if !path_str.is_empty() {
                        if path_str.contains(" → ") {
                            if let Some((transit, origin)) = path_str.rsplit_once(" → ") {
                                let transit_owned = format!("  {transit} → ");
                                let origin_owned = origin.to_string();
                                let origin_color = match pc.rpki {
                                    Some(crate::rpki::Status::Valid) => Color::Green,
                                    Some(crate::rpki::Status::InvalidWrongAsn) => Color::Yellow,
                                    _ => Color::DarkGray,
                                };
                                spans.push(Span::styled(transit_owned, Color::DarkGray));
                                spans.push(Span::styled(origin_owned, origin_color));
                            }
                        } else {
                            let color = match pc.rpki {
                                Some(crate::rpki::Status::Valid) => Color::Green,
                                Some(crate::rpki::Status::InvalidWrongAsn) => Color::Yellow,
                                _ => Color::DarkGray,
                            };
                            let owned = format!("  {path_str}");
                            spans.push(Span::styled(owned, color));
                        }
                    }
                    if let Some(ref hint) = rpki_hint {
                        spans.push(Span::from(format!("  ({hint})")).yellow());
                    }
                    // Origin AS name (last hop)
                    if let Some(origin) = display_path.last() {
                        let name = as_name_resolve(*origin);
                        if !name.is_empty() && !name.starts_with("AS") {
                            let truncated = if name.len() > 22 {
                                format!("{}…", &name[..21])
                            } else {
                                name
                            };
                            spans.push(Span::raw(" "));
                            spans.push(Span::styled(truncated, Color::Yellow));
                        }
                    }
                    // Show communities (max 3, only when toggled on)
                    if dash.show_communities && !pc.communities.is_empty() {
                        let shown = &pc.communities[..pc.communities.len().min(3)];
                        let comms = shown.join(" ");
                        let more = if pc.communities.len() > 3 {
                            format!(" +{}", pc.communities.len() - 3)
                        } else {
                            String::new()
                        };
                        spans.push(Span::from(format!("  {comms}{more}")).dark_gray());
                    }
                    lines.push(Line::from(spans));
                }
                for (p, _) in &pc.withdrawn {
                    let wd_path = dash
                        .prefix_last_path
                        .get(p)
                        .map(|path| strip_known_prefix(path, dash))
                        .unwrap_or_default();
                    let wd_path_str = compact_path(&wd_path);
                    let mut spans = vec![
                        Span::raw(format!("{ts} ")),
                        Span::styled(format!("{} ", rpki_str), rpki_color),
                        Span::styled(format!("{type_str:<8}"), type_color),
                        Span::raw(" -"),
                        Span::styled(p.as_str(), Color::Red),
                    ];
                    if !wd_path_str.is_empty() {
                        let label = format!("  (was: {wd_path_str})");
                        spans.push(Span::from(label).dark_gray());
                    }
                    lines.push(Line::from(spans));
                }
            }
            None => {
                let asn_str = msg
                    .asn
                    .map(|a| format!("AS{a}"))
                    .unwrap_or_else(|| "-".to_string());
                let ip_str = msg.ip_short.as_deref().unwrap_or("-");
                lines.push(Line::from(vec![
                    Span::raw(format!("{ts} ")),
                    Span::styled(format!("{type_str:<8}"), type_color),
                    Span::raw(format!("  {asn_str} {ip_str}")),
                ]));
            }
        }
    }

    // Only show the most recent lines that fit
    let visible: Vec<&Line> = if lines.len() > max_lines {
        lines[lines.len() - max_lines..].iter().collect()
    } else {
        lines.iter().collect()
    };

    let ticker = Paragraph::new(Text::from(
        visible.iter().map(|l| (*l).clone()).collect::<Vec<Line>>(),
    ))
    .block(Block::bordered().title("Live").borders(Borders::ALL));
    frame.render_widget(ticker, area);
}

struct PrefixChange {
    announced: Vec<(String, u32)>, // (prefix, origin_asn)
    withdrawn: Vec<(String, u32)>,
    as_path: Vec<u32>,        // full AS path
    communities: Vec<String>, // BGP communities as "ASN:VALUE" strings
    rpki: Option<crate::rpki::Status>,
    rpki_detail: Option<crate::rpki::RPKIDetail>,
}

/// Strip the known collector AS (6447) and the peer ASN from the displayed path.
/// The user is watching this peer — they don't need to see the first two hops.
fn strip_known_prefix(path: &[u32], dash: &Dashboard) -> Vec<u32> {
    let mut start = 0;
    // Strip 6447 (RouteViews collector)
    if start < path.len() && path[start] == 6447 {
        start += 1;
    }
    // Strip the peer ASN from the topic name
    if start < path.len() {
        let peer_asn = dash
            .topic
            .rsplit('.')
            .nth(1)
            .and_then(|s| s.parse::<u32>().ok());
        if let Some(peer) = peer_asn {
            if path[start] == peer {
                start += 1;
            }
        }
    }
    path[start..].to_vec()
}

fn extract_prefixes(full_data: &[u8]) -> Option<PrefixChange> {
    let mut data = Bytes::copy_from_slice(full_data);
    match bgpkit_parser::parser::bmp::parse_bmp_msg(&mut data) {
        Ok(msg) => {
            if let bgpkit_parser::parser::bmp::messages::BmpMessageBody::RouteMonitoring(rm) =
                msg.message_body
            {
                match rm.bgp_message {
                    bgpkit_parser::models::BgpMessage::Update(update) => {
                        let mut as_path: Vec<u32> = Vec::new();
                        let mut origin_asn: u32 = 0;
                        let mut communities: Vec<String> = Vec::new();
                        for attr in &update.attributes {
                            match attr {
                                bgpkit_parser::models::AttributeValue::AsPath {
                                    ref path, ..
                                } => {
                                    for seg in &path.segments {
                                        match seg {
                                            bgpkit_parser::models::AsPathSegment::AsSequence(v)
                                            | bgpkit_parser::models::AsPathSegment::AsSet(v) => {
                                                for a in v {
                                                    let n = a.to_u32();
                                                    as_path.push(n);
                                                    origin_asn = n;
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                bgpkit_parser::models::AttributeValue::Communities(ref comms) => {
                                    for c in comms {
                                        match c {
                                            bgpkit_parser::models::Community::Custom(asn, val) => {
                                                communities.push(format!(
                                                    "{}:{}",
                                                    asn.to_u32(),
                                                    val
                                                ));
                                            }
                                            bgpkit_parser::models::Community::NoExport => {
                                                communities.push("NO_EXPORT".into());
                                            }
                                            bgpkit_parser::models::Community::NoAdvertise => {
                                                communities.push("NO_ADVERTISE".into());
                                            }
                                            bgpkit_parser::models::Community::NoExportSubConfed => {
                                                communities.push("NO_EXPORT_SUBCONFED".into());
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        // Dedup and limit
                        communities.sort();
                        communities.dedup();
                        communities.truncate(8);

                        Some(PrefixChange {
                            announced: update
                                .announced_prefixes
                                .iter()
                                .map(|p| (p.to_string(), origin_asn))
                                .collect(),
                            withdrawn: update
                                .withdrawn_prefixes
                                .iter()
                                .map(|p| (p.to_string(), origin_asn))
                                .collect(),
                            as_path,
                            communities,
                            rpki: None,
                            rpki_detail: None,
                        })
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

fn render_origins(frame: &mut Frame, area: Rect, dash: &Dashboard) {
    if dash.prefix_origins.is_empty() {
        let placeholder = Paragraph::new(" waiting for AS path data...")
            .dark_gray()
            .block(
                Block::bordered()
                    .title("Prefix Flaps")
                    .borders(Borders::ALL),
            );
        frame.render_widget(placeholder, area);
        return;
    }

    // Sort origins by their single most-churned prefix (not total churn)
    let mut origin_max_churn: HashMap<u32, u64> = HashMap::new();
    for (prefix, origin) in &dash.prefix_origins {
        let churn = dash.churn_counts.get(prefix).copied().unwrap_or(0);
        let entry = origin_max_churn.entry(*origin).or_insert(0);
        *entry = (*entry).max(churn);
    }
    let mut origins: Vec<(u32, u64)> = origin_max_churn.into_iter().collect();
    origins.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let max_lines = (area.height.saturating_sub(3)) as usize;
    let mut lines: Vec<Line> = Vec::new();

    for (origin, _total_churn) in origins.iter().take(max_lines / 2) {
        // Collect all prefixes for this origin, sorted by churn
        let mut pfxs: Vec<(&String, u64)> = dash
            .prefix_origins
            .iter()
            .filter(|(_, o)| *o == origin)
            .map(|(p, _)| (p, dash.churn_counts.get(p).copied().unwrap_or(0)))
            .collect();
        pfxs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

        if pfxs.is_empty() {
            continue;
        }

        let name = as_name_resolve(*origin);
        let origin_label = if name.is_empty() || name.starts_with("AS") {
            format!("AS{origin}")
        } else {
            let truncated = if name.len() > 22 {
                format!("{}…", &name[..21])
            } else {
                name
            };
            format!("AS{origin} {truncated}")
        };

        let total_churn: u64 = pfxs.iter().map(|(_, c)| c).sum();
        let max_show = 4; // collapse long origin groups

        if pfxs.len() <= max_show {
            // Show all prefixes
            if pfxs.len() == 1 {
                let (p, _) = pfxs[0];
                lines.push(Line::from(vec![
                    Span::raw(format!("  {:<24} ", p)),
                    Span::raw(format!("─{:─>20}→ ", "")),
                    Span::raw(format!("{origin_label} ({total_churn})")),
                ]));
            } else {
                for (i, (p, c)) in pfxs.iter().enumerate() {
                    let connector = if i == 0 {
                        "──┐"
                    } else if i == pfxs.len() - 1 {
                        "──┘"
                    } else {
                        "──┤"
                    };
                    let line = if i == pfxs.len() / 2 {
                        Line::from(vec![
                            Span::raw(format!("  {:<24} ", p)),
                            Span::raw(format!("{connector} ")),
                            Span::raw(format!("─{:─>17}→ {} ({})", "", origin_label, total_churn)),
                        ])
                    } else {
                        Line::from(vec![
                            Span::raw(format!("  {:<24} ", p)),
                            Span::raw(format!("{connector:<6}")),
                            Span::from(format!("({c})")).dark_gray(),
                        ])
                    };
                    lines.push(line);
                }
            }
        } else {
            // Show first N prefixes, then a summary
            let rest = pfxs.len() - max_show;
            let rest_churn: u64 = pfxs[max_show..].iter().map(|(_, c)| c).sum();
            for (i, (p, c)) in pfxs.iter().take(max_show).enumerate() {
                let connector = if i == 0 { "──┐" } else { "──┤" };
                let line = if i == max_show / 2 {
                    Line::from(vec![
                        Span::raw(format!("  {:<24} ", p)),
                        Span::raw(format!("{connector} ")),
                        Span::raw(format!("─{:─>17}→ {} ({})", "", origin_label, total_churn)),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw(format!("  {:<24} ", p)),
                        Span::raw(format!("{connector:<6}")),
                        Span::from(format!("({c})")).dark_gray(),
                    ])
                };
                lines.push(line);
            }
            lines.push(Line::from(vec![
                Span::raw(format!("  {:<24} ", "")),
                Span::raw("──┘ ".to_string()),
                Span::from(format!("... +{rest} more prefixes (+{rest_churn} churn)")).dark_gray(),
            ]));
        }
        lines.push(Line::from(""));
        if lines.len() >= max_lines {
            break;
        }
    }

    let panel = Paragraph::new(Text::from(lines)).block(
        Block::bordered()
            .title("Prefix Flaps")
            .borders(Borders::ALL),
    );
    frame.render_widget(panel, area);
}

/// Format an AS path compactly: deduplicate repeats, truncate long paths,
/// bake origin name into last hop. Returns "A → B → … → Z" or similar.
fn compact_path(path: &[u32]) -> String {
    if path.is_empty() {
        return String::new();
    }

    // Deduplicate consecutive repeats: [A, B, B, B, C] → [A, B×3, C]
    let mut deduped: Vec<(u32, u32)> = Vec::new();
    for &asn in path {
        match deduped.last_mut() {
            Some((last, count)) if *last == asn => *count += 1,
            _ => deduped.push((asn, 1)),
        }
    }

    let as_str = |asn: u32, count: u32| -> String {
        if count > 1 {
            format!("AS{asn}×{count}")
        } else {
            format!("AS{asn}")
        }
    };

    if deduped.len() <= 4 {
        deduped
            .iter()
            .map(|&(asn, count)| as_str(asn, count))
            .collect::<Vec<_>>()
            .join(" → ")
    } else {
        // First 1 + … + last 1
        let mut parts: Vec<String> = Vec::new();
        parts.push(as_str(deduped[0].0, deduped[0].1));
        parts.push("…".into());
        let last = deduped.len() - 1;
        parts.push(as_str(deduped[last].0, deduped[last].1));
        parts.join(" → ")
    }
}

fn msg_type_label(t: u8) -> &'static str {
    match t {
        0 => "PFX",
        1 => "STATS",
        2 => "PEER_DN",
        3 => "PEER_UP",
        4 => "INIT",
        5 => "TERM",
        6 => "MIRROR",
        _ => "?",
    }
}

fn msg_type_color(t: u8) -> Color {
    match t {
        0 => Color::Cyan,
        1 => Color::Blue,
        2 => Color::Red,
        3 => Color::Green,
        4 | 5 => Color::Yellow,
        6 => Color::Magenta,
        _ => Color::Gray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    // -----------------------------------------------------------------------
    // strip_known_prefix
    // -----------------------------------------------------------------------

    #[test]
    fn test_strip_known_prefix_strips_6447_and_peer_asn() {
        let dash = Dashboard::new("routeviews.route-views2.2152.bmp_raw", 10);
        let path = vec![6447, 2152, 3356, 16509];
        let result = strip_known_prefix(&path, &dash);
        assert_eq!(result, vec![3356, 16509]);
    }

    #[test]
    fn test_strip_known_prefix_strips_only_collector_peer_topology() {
        let dash = Dashboard::new("routeviews.linx.11537.bmp_raw", 10);
        let path = vec![6447, 11537, 16509];
        let result = strip_known_prefix(&path, &dash);
        assert_eq!(result, vec![16509]);
    }

    #[test]
    fn test_strip_known_prefix_no_collector_prefix_preserves_path() {
        let dash = Dashboard::new("routeviews.route-views2.2152.bmp_raw", 10);
        let path = vec![3356, 16509];
        let result = strip_known_prefix(&path, &dash);
        assert_eq!(result, vec![3356, 16509]);
    }

    // -----------------------------------------------------------------------
    // extract_prefixes
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_prefixes_does_not_panic() {
        // Use the raw_bmp fixture to build a minimal route monitoring frame.
        // The BGP payload inside the fixture is intentionally minimal and
        // may not parse as a valid UPDATE; the important thing is that
        // extract_prefixes never panics.
        let frame =
            crate::raw_bmp::fixtures::make_route_monitoring_frame(13335, [10, 0, 0, 1], 1000, 0);
        // Should return Some or None but never panic
        let _result = extract_prefixes(&frame);
    }

    #[test]
    fn test_extract_prefixes_with_proper_bgp_update() {
        use crate::raw_bmp::fixtures;

        // Build a proper BGP UPDATE message with:
        //   AS_PATH: [6447, 13335] (32-bit AS numbers)
        //   Announced: 10.0.0.0/24
        //   Origin: 13335

        // ---------- BGP wire-format message ----------
        let mut bgp = Vec::new();

        // Marker (16 bytes of 0xFF per RFC 4271)
        bgp.extend_from_slice(&[0xFF; 16]);
        // Length placeholder (will patch below)
        bgp.extend_from_slice(&[0x00, 0x00]);
        // Type = UPDATE
        bgp.push(0x02);

        // UPDATE body
        // Withdrawn routes length = 0
        bgp.extend_from_slice(&[0x00, 0x00]);

        // --- Path attributes (32-bit AS numbers) ---
        let mut attrs = Vec::new();

        // ORIGIN: IGP (Well-known, Transitive, Complete)
        attrs.extend_from_slice(&[0x40, 0x01, 0x01, 0x00]);

        // AS_PATH: AS_SEQUENCE [6447, 13335]
        // flags=0x40 type=2, length=10, AS_SEQUENCE(2), count=2
        attrs.extend_from_slice(&[
            0x40, 0x02, 0x0A, 0x02, 0x02, 0x00, 0x00, 0x19, 0x2F, // 6447
            0x00, 0x00, 0x34, 0x17, // 13335
        ]);

        // NEXT_HOP: 10.0.0.1
        attrs.extend_from_slice(&[0x40, 0x03, 0x04, 10, 0, 0, 1]);

        // Attribute length
        let attr_len = attrs.len() as u16; // 24
        bgp.extend_from_slice(&attr_len.to_be_bytes());
        bgp.extend_from_slice(&attrs);

        // NLRI: 10.0.0.0/24
        bgp.extend_from_slice(&[24, 10, 0, 0]);

        // Patch BGP message length (header = 16 marker + 2 len + 1 type + body)
        let bgp_total = bgp.len() as u16;
        bgp[16..18].copy_from_slice(&bgp_total.to_be_bytes());

        // ---------- BMP Route Monitoring frame ----------
        let pph = fixtures::make_per_peer_header(13335, [10, 0, 0, 1], 1000, 0);
        let total_payload: Vec<u8> = pph.into_iter().chain(bgp).collect();
        let bmp_header = fixtures::make_common_header(0, total_payload.len() as u32);
        let full_data: Vec<u8> = bmp_header.into_iter().chain(total_payload).collect();

        // Act
        let result = extract_prefixes(&full_data);

        // Assert
        assert!(result.is_some(), "valid RM frame should parse to Some");
        let pc = result.unwrap();

        assert!(!pc.announced.is_empty(), "should have announced prefixes");
        assert_eq!(pc.announced[0].0, "10.0.0.0/24");
        assert_eq!(pc.announced[0].1, 13335); // origin ASN

        assert_eq!(pc.as_path, vec![6447, 13335], "AS path");

        assert!(pc.withdrawn.is_empty(), "no withdrawn prefixes");
    }

    // -----------------------------------------------------------------------
    // as_name_resolve
    // -----------------------------------------------------------------------

    #[test]
    fn test_as_name_resolve_known_asn_returns_seed_name() {
        assert_eq!(as_name_resolve(13335), "Cloudflare");
    }

    #[test]
    fn test_as_name_resolve_unknown_returns_raw_asn() {
        let name = as_name_resolve(999_999);
        assert_eq!(name, "AS999999");
        // Clean up global cache side effect
        global_name_cache().lock().unwrap().remove(&999_999);
    }

    #[test]
    fn test_as_name_resolve_global_cache_shared() {
        assert_eq!(as_name_resolve(13335), "Cloudflare");
        assert_eq!(as_name_resolve(13335), "Cloudflare");
    }

    #[test]
    fn test_as_name_cymru_seed_resolves_unknown() {
        // AS11537 is in the Cymru seed ("Internet2, US") but not curated seed.
        let name = as_name_resolve(11537);
        assert!(
            !name.starts_with("AS"),
            "AS11537 should resolve from Cymru seed, got {name}"
        );
        // Clean up global cache side effect
        global_name_cache().lock().unwrap().remove(&11537);
    }

    // -----------------------------------------------------------------------
    // parse_topic
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_topic_normal() {
        let result = crate::browser::parse_topic("routeviews.chicago.13335.bmp_raw");
        assert!(result.is_some(), "expected parse success");
        let pt = result.unwrap();
        assert_eq!(pt.collector, "chicago");
        assert_eq!(pt.asn_str, "13335");
        assert_eq!(pt.full, "routeviews.chicago.13335.bmp_raw");
    }

    #[test]
    fn test_parse_topic_collector_with_dashes() {
        let result = crate::browser::parse_topic("routeviews.route-views2.2152.bmp_raw");
        assert!(result.is_some());
        let pt = result.unwrap();
        assert_eq!(pt.collector, "route-views2");
        assert_eq!(pt.asn_str, "2152");
    }

    #[test]
    fn test_parse_topic_not_a_valid_topic() {
        let result = crate::browser::parse_topic("not.a.valid.topic");
        assert!(result.is_none(), "expected None for non-matching topic");
    }

    #[test]
    fn test_parse_topic_no_asn_component() {
        // routeviews.<collector>.bmp_raw with no ASN segment
        let result = crate::browser::parse_topic("routeviews.chicago.bmp_raw");
        assert!(result.is_some(), "edge case should still parse");
        let pt = result.unwrap();
        assert_eq!(pt.collector, "chicago");
        assert_eq!(pt.asn_str, "-");
    }

    // -----------------------------------------------------------------------
    // msg_type_label / msg_type_color
    // -----------------------------------------------------------------------

    #[test]
    fn test_msg_type_label_all_known_types() {
        let cases: [(u8, &str); 7] = [
            (0, "PFX"),
            (1, "STATS"),
            (2, "PEER_DN"),
            (3, "PEER_UP"),
            (4, "INIT"),
            (5, "TERM"),
            (6, "MIRROR"),
        ];
        for (t, expected) in &cases {
            assert_eq!(msg_type_label(*t), *expected, "type {}", t);
        }
    }

    #[test]
    fn test_msg_type_label_unknown_types_fallback() {
        assert_eq!(msg_type_label(7), "?");
        assert_eq!(msg_type_label(255), "?");
    }

    #[test]
    fn test_msg_type_color_all_known_types() {
        use ratatui::style::Color;

        assert_eq!(msg_type_color(0), Color::Cyan);
        assert_eq!(msg_type_color(1), Color::Blue);
        assert_eq!(msg_type_color(2), Color::Red);
        assert_eq!(msg_type_color(3), Color::Green);
        assert_eq!(msg_type_color(4), Color::Yellow);
        assert_eq!(msg_type_color(5), Color::Yellow);
        assert_eq!(msg_type_color(6), Color::Magenta);
    }

    #[test]
    fn test_msg_type_color_unknown_types_fallback() {
        assert_eq!(msg_type_color(7), Color::Gray);
        assert_eq!(msg_type_color(255), Color::Gray);
    }

    // -----------------------------------------------------------------------
    // Legacy — existing render test
    // -----------------------------------------------------------------------

    #[test]
    fn test_dashboard_render_no_panic() {
        let mut dash = Dashboard::new("test-topic", 10);

        // Feed a few synthetic messages
        dash.rolling.push(0, false, vec![], None);
        dash.rolling.push(3, false, vec![], None);
        dash.rolling.push(0, false, vec![], None);
        dash.rolling.push(2, true, vec![], None);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, &dash, true))
            .expect("render should not panic");
    }

    #[test]
    fn test_dashboard_render_waiting_state_no_panic() {
        let dash = Dashboard::new("test-topic", 10);
        // Zero messages — connected = false
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, &dash, false))
            .expect("render with no messages should not panic");
    }

    #[test]
    fn test_render_loading_no_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_loading(f, f.area(), "test-topic", "broker:9092"))
            .expect("render_loading should not panic");
    }

    #[test]
    fn test_keepalive_count_starts_zero() {
        let dash = Dashboard::new("test", 10);
        assert_eq!(dash.keepalive_count, 0);
    }

    #[test]
    fn test_render_no_keepalive_text() {
        let mut dash = Dashboard::new("test", 10);
        dash.keepalive_count = 5;
        dash.rolling.push(0, false, vec![], None);
        dash.total_messages = 6;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &dash, true)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(!text.contains("(keepalive)"));
    }

    #[test]
    fn test_render_waiting_connecting_before_10s() {
        let dash = Dashboard::new("test", 10);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &dash, false)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("Connecting"));
        assert!(text.contains("connecting..."));
    }

    #[test]
    fn test_render_waiting_no_messages_after_10s() {
        let mut dash = Dashboard::new("test", 10);
        dash.session_start = Instant::now() - Duration::from_secs(11);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &dash, false)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("No messages yet"));
        assert!(text.contains("no messages yet"));
    }

    #[test]
    fn test_normalize_cymru_strips_asn_prefix() {
        assert_eq!(
            normalize_cymru_name(1916, "AS1916 - Rede Nacional..."),
            "Rede Nacional..."
        );
        assert_eq!(
            normalize_cymru_name(1916, "AS1916 Rede Nacional..."),
            "Rede Nacional..."
        );
    }

    #[test]
    fn test_normalize_cymru_strips_handle_prefix() {
        assert_eq!(
            normalize_cymru_name(174, "COGENT-174 - Cogent Communications, LLC, US"),
            "Cogent Communications, LLC, US"
        );
    }

    #[test]
    fn test_normalize_cymru_preserves_nonmatching() {
        assert_eq!(
            normalize_cymru_name(1916, "Rede Nacional..."),
            "Rede Nacional..."
        );
    }

    // ── RouteViews peer metadata tests ──

    #[test]
    fn test_format_prefix_count() {
        assert_eq!(format_prefix_count(520), "520 pfx");
        assert_eq!(format_prefix_count(950_000), "950k pfx");
        assert_eq!(format_prefix_count(1_044_754), "1.0M pfx");
        assert_eq!(format_prefix_count(10_500_000), "10M pfx");
        assert_eq!(format_prefix_count(0), "0 pfx");
    }

    #[test]
    fn test_routeviews_peers_loads() {
        let peers = routeviews_peers();
        // 1921 peer sessions → ~830 unique (collector, asn) pairs (IPv4+IPv6 dedup)
        assert!(
            peers.len() > 500,
            "expected >500 peers, got {}",
            peers.len()
        );
    }

    #[test]
    fn test_routeviews_prefix_count_known() {
        // amsix.ams + AS1103 = SURFnet, ~1M prefixes (IPv6 entry may overwrite IPv4)
        let pfx = routeviews_prefix_count("amsix.ams", 1103);
        assert!(pfx.is_some(), "amsix.ams+AS1103 should be in peer data");
        assert!(pfx.unwrap() > 100_000);
    }

    #[test]
    fn test_routeviews_prefix_count_unknown() {
        assert_eq!(routeviews_prefix_count("nonexistent", 99999), None);
    }

    #[test]
    fn test_routeviews_peer_name_lookup() {
        let name = routeviews_peer_name("amsix.ams", 1103);
        assert!(name.is_some(), "amsix.ams+AS1103 should have a peer name");
        let n = name.unwrap().to_lowercase();
        assert!(
            n.contains("surf"),
            "expected SURFnet-related name, got: {n}"
        );
    }

    #[test]
    fn test_routeviews_asn_fallback() {
        let fallback = routeviews_asn_fallback();
        // AS1103 should be in the fallback map
        assert!(fallback.contains_key(&1103));
    }

    #[test]
    fn test_routeviews_fallback_populated() {
        let fallback = routeviews_asn_fallback();
        // Should contain hundreds of ASNs from peering-status.html
        assert!(fallback.len() > 200);
    }

    #[test]
    fn test_duplicate_peer_rows_keep_max_prefixes() {
        // route-views3 + AS29479 has two rows in bundled TSV: 952511 and 2
        let pfx = routeviews_prefix_count("route-views3", 29479);
        assert!(pfx.is_some(), "route-views3+AS29479 should be in peer data");
        assert!(
            pfx.unwrap() >= 900_000,
            "should keep max-prefix row (952511), got: {}",
            pfx.unwrap()
        );
    }

    #[test]
    fn test_as29479_name_not_overwritten_by_ipv6() {
        // The IPv6 row for AS29479 may have a slightly different name.
        // Max-prefix row (952511) name should be preferred.
        let name = routeviews_peer_name("route-views3", 29479);
        assert!(name.is_some());
        // The large-table row name should contain the main org name
        let n = name.unwrap().to_lowercase();
        assert!(
            n.contains("transdata"),
            "expected Transdata name from max-prefix row, got: {n}"
        );
    }

    #[test]
    fn test_asn_fallback_keeps_max_prefix_name() {
        // For AS29479, the fallback name should come from the max-prefix entry
        let fallback = routeviews_asn_fallback();
        let name = fallback.get(&29479);
        assert!(name.is_some(), "AS29479 should be in fallback");
        assert!(
            name.unwrap().to_lowercase().contains("transdata"),
            "fallback should use max-prefix name"
        );
    }
}
