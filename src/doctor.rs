use std::path::Path;

use bytes::Bytes;

use crate::error::DoctorError;
use crate::event::{emit_jsonl, JsonlEvent};
use crate::input::{self, InputFormat};
use crate::lint;
use crate::obmp_reader::{ContainerStats, ObmpReader};
use crate::raw_bmp::{
    BmpMessageType, PerPeerHeader, RawBmpFrame, RawBmpIterator, BMP_EXPECTED_VERSION,
    BMP_PER_PEER_HEADER_SIZE,
};
use crate::state::{DoctorState, Finding, PeerKey, Severity};

pub struct Doctor {
    pub state: DoctorState,
    pub events: Vec<JsonlEvent>,
    max_findings: usize,
    findings_truncated: bool,
    format: InputFormat,
}

const DEFAULT_MAX_FINDINGS: usize = 1000;

enum FrameSource {
    RawBmp(RawBmpIterator),
    Obmp(ObmpReader),
}

impl FrameSource {
    fn container_stats(&self) -> Option<&ContainerStats> {
        match self {
            FrameSource::Obmp(reader) => Some(&reader.stats),
            FrameSource::RawBmp(_) => None,
        }
    }
}

impl Iterator for FrameSource {
    type Item = Result<RawBmpFrame, DoctorError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            FrameSource::RawBmp(iter) => iter.next(),
            FrameSource::Obmp(reader) => reader.next(),
        }
    }
}

impl Doctor {
    #[allow(dead_code)]
    pub fn new(file_path: &Path) -> Result<Self, DoctorError> {
        Self::with_max_findings(file_path, DEFAULT_MAX_FINDINGS, InputFormat::RawBmp)
    }

    pub fn with_max_findings(
        file_path: &Path,
        max_findings: usize,
        format: InputFormat,
    ) -> Result<Self, DoctorError> {
        let (file_size, format_str) = input::file_size_and_format(file_path, format)?;
        let state = DoctorState {
            file_path: file_path.to_string_lossy().to_string(),
            file_size,
            format: format_str,
            ..Default::default()
        };
        Ok(Doctor {
            state,
            events: Vec::new(),
            max_findings,
            findings_truncated: false,
            format,
        })
    }

    pub fn was_truncated(&self) -> bool {
        self.findings_truncated
    }

    fn push_finding(&mut self, finding: Finding) {
        if self.state.findings.len() >= self.max_findings {
            self.findings_truncated = true;
            self.state.findings_dropped += 1;
            return;
        }
        self.state.findings.push(finding);
    }

    pub fn process(&mut self, collect_events: bool) -> Result<(), DoctorError> {
        let mut iter: FrameSource = match self.format {
            InputFormat::RawBmp => {
                FrameSource::RawBmp(RawBmpIterator::open(&self.state.file_path)?)
            }
            InputFormat::Bmpd => FrameSource::Obmp(ObmpReader::open(&self.state.file_path)?),
            InputFormat::Auto => unreachable!("Auto should be resolved before Doctor construction"),
        };

        for frame_result in &mut iter {
            match frame_result {
                Ok(frame) => {
                    self.process_frame(frame, collect_events);
                }
                Err(DoctorError::Frame(msg)) => {
                    self.state.malformed_messages += 1;
                    let finding = Finding {
                        severity: Severity::Error,
                        rule: lint::RULE_TRUNCATED_FRAME.to_string(),
                        offset: None,
                        peer: None,
                        message: msg.clone(),
                    };
                    self.push_finding(finding.clone());
                    if collect_events {
                        let event = JsonlEvent::from_frame(
                            self.state.total_messages,
                            None,
                            0,
                            None,
                            None,
                            0,
                            "malformed",
                            0,
                            None,
                            None,
                            &[finding],
                        );
                        self.events.push(event);
                    }
                }
                Err(e) => return Err(e),
            }
        }

        // Copy container stats for --summary-json output
        if let Some(stats) = iter.container_stats() {
            self.state.container_stats = stats.clone();
        }

        Ok(())
    }

    fn process_frame(&mut self, frame: RawBmpFrame, collect_events: bool) {
        self.state.total_messages += 1;
        *self.state.by_type.entry(frame.msg_type_raw).or_insert(0) += 1;

        let mut frame_findings: Vec<Finding> = Vec::new();

        if frame.version != BMP_EXPECTED_VERSION {
            frame_findings.push(lint::finding_invalid_version(frame.offset, frame.version));
        }

        let msg_type = frame.msg_type;
        if msg_type.is_none() {
            frame_findings.push(lint::finding_unknown_type(frame.offset, frame.msg_type_raw));
        }

        let peer_key = frame.per_peer_header.as_ref().map(peer_key_from_pph);

        let mut bgp_elems_count: u64 = 0;
        let mut parse_ok = true;

        if frame_findings.is_empty() && msg_type.is_some() {
            match try_bgpkit_parse(&frame.full_data) {
                Ok(count) => {
                    bgp_elems_count = count;
                    self.state.bgp_elem_count += count;
                }
                Err(err_msg) => {
                    parse_ok = false;
                    frame_findings.push(lint::finding_parse_error(
                        frame.offset,
                        peer_key.clone(),
                        err_msg,
                    ));
                }
            }
        }

        // Update peer state and generate peer-related findings
        if let (Some(ref pk), Some(ref pph), Some(mt)) =
            (&peer_key, &frame.per_peer_header, msg_type)
        {
            let peer = self.state.peers.entry(pk.clone()).or_default();
            let curr_ts = (pph.timestamp_seconds, pph.timestamp_microseconds);

            // Check timestamp regression
            if let Some(last_ts) = peer.last_timestamp {
                if is_timestamp_before(curr_ts, last_ts) {
                    peer.timestamp_regression_count += 1;
                    frame_findings.push(lint::finding_timestamp_regression(
                        frame.offset,
                        pk.clone(),
                        last_ts.0,
                        last_ts.1,
                        curr_ts.0,
                        curr_ts.1,
                    ));
                }
            }

            peer.last_timestamp = Some(curr_ts);
            if peer.first_timestamp.is_none() {
                peer.first_timestamp = Some(curr_ts);
            }

            match mt {
                BmpMessageType::RouteMonitoring | BmpMessageType::RouteMirroringMessage => {
                    peer.route_monitoring_count += 1;
                    if !peer.peer_up_seen {
                        peer.update_before_peer_up_count += 1;
                        frame_findings.push(lint::finding_route_monitoring_before_peer_up(
                            frame.offset,
                            pk.clone(),
                        ));
                    }
                }
                BmpMessageType::PeerUpNotification => {
                    peer.peer_up_count += 1;
                    if peer.active {
                        frame_findings
                            .push(lint::finding_duplicate_peer_up(frame.offset, pk.clone()));
                    } else {
                        peer.peer_up_seen = true;
                        peer.active = true;
                    }
                }
                BmpMessageType::PeerDownNotification => {
                    peer.peer_down_count += 1;
                    // RFC 7854, Section 3.5: Peer Down Notification payload
                    // begins with a 1-byte Reason code immediately after the
                    // Per-Peer Header. IANA BMP Parameters registry defines
                    // the values: 0=reserved, 1=Local system closed,
                    // 2=Local system closed (admin), 3=Remote system closed,
                    // 4=Remote notification, 5=Peer de-configured, 6=Local
                    // system terminated, 7=Local system exhausted resources.
                    let reason = frame
                        .payload
                        .get(BMP_PER_PEER_HEADER_SIZE)
                        .copied()
                        .unwrap_or(0);
                    peer.last_peer_down_reason = Some(reason);
                    if !peer.active {
                        frame_findings.push(lint::finding_peer_down_without_peer_up(
                            frame.offset,
                            pk.clone(),
                            reason,
                        ));
                    } else {
                        peer.active = false;
                    }
                }
                _ => {
                    peer.other_message_count += 1;
                }
            }
        }

        // Collect Initiation / Termination TLV info from the first
        // such message encountered.
        if let Some(ref tlv) = frame.tlv_info {
            match frame.msg_type {
                Some(BmpMessageType::InitiationMessage) if self.state.initiation_info.is_none() => {
                    self.state.initiation_info = Some(tlv.clone());
                }
                Some(BmpMessageType::TerminationMessage)
                    if self.state.termination_info.is_none() =>
                {
                    self.state.termination_info = Some(tlv.clone());
                }
                _ => {}
            }
        }

        // Collect Stats Report info from the first such message
        if let Some(ref stats) = frame.stats_info {
            if self.state.stats_info.is_none() && !stats.entries.is_empty() {
                self.state.stats_info = Some(stats.clone());
            }
        }

        for f in &frame_findings {
            self.push_finding(f.clone());
        }

        if collect_events {
            let parse_status = if parse_ok { "ok" } else { "error" };
            let ts = frame
                .per_peer_header
                .as_ref()
                .map(|pph| (pph.timestamp_seconds, pph.timestamp_microseconds));
            let event = JsonlEvent::from_frame(
                frame.offset,
                msg_type,
                frame.msg_type_raw,
                peer_key.as_ref(),
                ts,
                frame.msg_len,
                parse_status,
                bgp_elems_count,
                frame.tlv_info.as_ref(),
                frame.stats_info.as_ref(),
                &frame_findings,
            );
            self.events.push(event);
        }
    }

    pub fn dump_jsonl(&self) {
        emit_jsonl(&self.events);
    }
}

fn peer_key_from_pph(pph: &PerPeerHeader) -> PeerKey {
    let ip = pph.peer_ip();
    PeerKey {
        peer_asn: Some(pph.peer_asn),
        peer_ip: Some(ip),
        peer_distinguisher: if pph.peer_type != 0 {
            Some(hex::encode(pph.peer_distinguisher))
        } else {
            None
        },
    }
}

fn try_bgpkit_parse(frame_data: &[u8]) -> Result<u64, String> {
    let mut bytes = Bytes::copy_from_slice(frame_data);
    match bgpkit_parser::parser::bmp::parse_bmp_msg(&mut bytes) {
        Ok(msg) => {
            let count = match msg.message_body {
                bgpkit_parser::parser::bmp::messages::BmpMessageBody::RouteMonitoring(rm) => {
                    count_bgp_elems_in_update(rm.bgp_message)
                }
                _ => 0,
            };
            Ok(count)
        }
        Err(e) => Err(format!("{e}")),
    }
}

fn count_bgp_elems_in_update(bgp_msg: bgpkit_parser::models::BgpMessage) -> u64 {
    match bgp_msg {
        bgpkit_parser::models::BgpMessage::Update(update) => {
            let announced = update.announced_prefixes.len() as u64;
            let withdrawn = update.withdrawn_prefixes.len() as u64;
            announced + withdrawn
        }
        _ => 0,
    }
}

fn is_timestamp_before(a: (u32, u32), b: (u32, u32)) -> bool {
    a.0 < b.0 || (a.0 == b.0 && a.1 < b.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw_bmp::fixtures;
    use std::io::Write;

    #[test]
    fn test_doctor_process_valid_session() {
        let peer_up = fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let rm = fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 200, 0);
        let peer_down = fixtures::make_peer_down_frame(65000, [10, 0, 0, 1], 300, 0, 3);
        let data = fixtures::write_fixture(&[peer_up, rm, peer_down]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 3);
        assert_eq!(*doctor.state.by_type.get(&0).unwrap_or(&0), 1);
        assert_eq!(*doctor.state.by_type.get(&3).unwrap_or(&0), 1);
        assert_eq!(*doctor.state.by_type.get(&2).unwrap_or(&0), 1);
        assert_eq!(doctor.state.malformed_messages, 0);
        assert_eq!(doctor.state.peers.len(), 1);

        let peer = doctor.state.peers.values().next().unwrap();
        assert_eq!(peer.peer_up_count, 1);
        assert_eq!(peer.route_monitoring_count, 1);
        assert_eq!(peer.peer_down_count, 1);
        assert!(!peer.active);

        // Ignore parse errors from bgpkit-parser on synthetic BGP data.
        // Core diagnostics: no peer lifecycle warnings.
        let peer_lint_rules: Vec<_> = doctor
            .state
            .findings
            .iter()
            .filter(|f| {
                f.rule == "route_monitoring_before_peer_up"
                    || f.rule == "duplicate_peer_up"
                    || f.rule == "peer_down_without_peer_up"
                    || f.rule == "timestamp_regression"
            })
            .collect();
        assert!(
            peer_lint_rules.is_empty(),
            "Unexpected peer lifecycle findings: {:?}",
            peer_lint_rules
        );
    }

    #[test]
    fn test_doctor_route_monitoring_before_peer_up() {
        let rm = fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 100, 0);
        let peer_up = fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 200, 0);
        let data = fixtures::write_fixture(&[rm, peer_up]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 2);
        assert!(!doctor.state.findings.is_empty());
        let has_rm_before_up = doctor
            .state
            .findings
            .iter()
            .any(|f| f.rule == "route_monitoring_before_peer_up");
        assert!(has_rm_before_up);
    }

    #[test]
    fn test_doctor_duplicate_peer_up() {
        let peer_up1 = fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let peer_up2 = fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 200, 0);
        let data = fixtures::write_fixture(&[peer_up1, peer_up2]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 2);
        let has_dup = doctor
            .state
            .findings
            .iter()
            .any(|f| f.rule == "duplicate_peer_up");
        assert!(has_dup);
    }

    #[test]
    fn test_doctor_peer_down_without_up() {
        let peer_down = fixtures::make_peer_down_frame(65000, [10, 0, 0, 1], 100, 0, 3);
        let data = peer_down;

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 1);
        let has_down_without_up = doctor
            .state
            .findings
            .iter()
            .any(|f| f.rule == "peer_down_without_peer_up");
        assert!(has_down_without_up);
    }

    #[test]
    fn test_doctor_timestamp_regression() {
        let peer_up = fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 300, 0);
        let rm = fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 200, 0);
        let data = fixtures::write_fixture(&[peer_up, rm]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 2);
        let has_regression = doctor
            .state
            .findings
            .iter()
            .any(|f| f.rule == "timestamp_regression");
        assert!(has_regression);
    }

    #[test]
    fn test_doctor_invalid_version() {
        let bad = fixtures::make_invalid_version_frame();

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&bad).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        let has_invalid_ver = doctor
            .state
            .findings
            .iter()
            .any(|f| f.rule == "invalid_bmp_version");
        assert!(has_invalid_ver);
    }

    #[test]
    fn test_doctor_malformed_frame() {
        let bad = fixtures::make_truncated_frame();

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&bad).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.malformed_messages, 1);
        let has_truncated = doctor
            .state
            .findings
            .iter()
            .any(|f| f.rule == "truncated_frame");
        assert!(has_truncated);
    }

    #[test]
    fn test_doctor_dump_jsonl() {
        let peer_up = fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let rm = fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 200, 0);
        let data = fixtures::write_fixture(&[peer_up, rm]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(true).unwrap();

        assert_eq!(doctor.events.len(), 2);
        assert_eq!(doctor.events[0].bmp_type_id, 3); // peer up
        assert_eq!(doctor.events[1].bmp_type_id, 0); // route monitoring
    }

    #[test]
    fn test_doctor_multiple_peers() {
        let pu1 = fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let pu2 = fixtures::make_peer_up_frame(65001, [10, 0, 0, 2], 200, 0);
        let rm1 = fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 300, 0);
        let rm2 = fixtures::make_route_monitoring_frame(65001, [10, 0, 0, 2], 400, 0);
        let data = fixtures::write_fixture(&[pu1, pu2, rm1, rm2]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 4);
        assert_eq!(doctor.state.peers.len(), 2);

        let active_count = doctor.state.peers.values().filter(|p| p.active).count();
        assert_eq!(active_count, 2);
    }

    #[test]
    fn test_doctor_initiation_message() {
        let init = fixtures::make_initiation_frame("test-router");
        let data = init;

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 1);
        assert_eq!(*doctor.state.by_type.get(&4).unwrap_or(&0), 1);
        assert_eq!(doctor.state.peers.len(), 0);
    }

    #[test]
    fn test_multi_frame_two_valid() {
        let pu = fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let rm = fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 200, 0);
        let data = fixtures::write_fixture(&[pu, rm]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 2);
        assert_eq!(doctor.state.peers.len(), 1);
        assert_eq!(doctor.state.malformed_messages, 0);
    }

    #[test]
    fn test_multi_frame_valid_then_truncated() {
        let pu = fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let bad = fixtures::make_truncated_frame();
        let data = [pu, bad].concat();

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 1);
        assert_eq!(doctor.state.malformed_messages, 1);
        assert_eq!(doctor.state.peers.len(), 1);
    }

    #[test]
    fn test_multi_frame_malformed_then_extra_bytes() {
        let bad = fixtures::make_invalid_length_frame();
        let extra = vec![0x00, 0x01, 0x02, 0x03]; // trailing garbage

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&bad).unwrap();
        tmp.write_all(&extra).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.malformed_messages, 1);
        assert!(doctor
            .state
            .findings
            .iter()
            .any(|f| f.rule == "truncated_frame"));
    }

    #[test]
    fn test_route_monitoring_before_any_peer_up() {
        let rm = fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 100, 0);
        let rm2 = fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 200, 0);
        let pu = fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 300, 0);
        let data = fixtures::write_fixture(&[rm, rm2, pu]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::new(&path).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 3);
        let rm_before_up_count = doctor
            .state
            .findings
            .iter()
            .filter(|f| f.rule == "route_monitoring_before_peer_up")
            .count();
        assert_eq!(rm_before_up_count, 2);

        let peer = doctor.state.peers.values().next().unwrap();
        assert_eq!(peer.update_before_peer_up_count, 2);
        assert!(peer.active);
    }

    #[test]
    fn test_max_findings_cap() {
        let bad1 = fixtures::make_invalid_version_frame();
        let bad2 = fixtures::make_invalid_version_frame();
        let bad3 = fixtures::make_invalid_version_frame();
        let data = fixtures::write_fixture(&[bad1, bad2, bad3]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        // Each invalid-version frame generates 1 finding. Cap at 2.
        let mut doctor = Doctor::with_max_findings(&path, 2, InputFormat::RawBmp).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.findings.len(), 2);
        assert!(doctor.was_truncated());
        assert_eq!(doctor.state.findings_dropped, 1);
    }

    #[test]
    fn test_max_findings_cap_zero() {
        let bad = fixtures::make_invalid_version_frame();

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&bad).unwrap();
        let path = tmp.into_temp_path();

        // Cap at 0: all findings dropped.
        let mut doctor = Doctor::with_max_findings(&path, 0, InputFormat::RawBmp).unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.findings.len(), 0);
        assert!(doctor.was_truncated());
        assert_eq!(doctor.state.findings_dropped, 1);
    }

    #[test]
    fn test_override_raw_bmp_on_obmp_file() {
        // .bmpd file with --format raw-bmp should fail as raw BMP,
        // not silently auto-correct to bmpd
        let inner = crate::raw_bmp::fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let wrapped = crate::obmp_reader::fixtures::make_openbmp_wrapped(&inner);
        let data = crate::obmp_reader::fixtures::make_valid_obmp(&[wrapped]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::with_max_findings(&path, 1000, InputFormat::RawBmp).unwrap();
        doctor.process(false).unwrap();

        // The raw BMP parser sees the .bmpd magic as garbage
        assert!(doctor.state.malformed_messages >= 1);
        assert_eq!(doctor.state.peers.len(), 0);
    }

    #[test]
    fn test_override_openbmp_len_on_raw_bmp_file() {
        // Raw BMP file with --format bmpd should fail as obmp,
        // not silently auto-correct to raw-bmp
        let frame = crate::raw_bmp::fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let data = crate::raw_bmp::fixtures::write_fixture(&[frame]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut doctor = Doctor::with_max_findings(&path, 1000, InputFormat::Bmpd).unwrap();
        // ObmpReader fails on missing magic
        assert!(doctor.process(false).is_err());
    }

    #[test]
    fn test_init_term_tlv_fixture() {
        // Read-only regression test against committed fixture
        let mut doctor = Doctor::with_max_findings(
            Path::new("tests/fixtures/init-term-tlvs.bmpd"),
            1000,
            InputFormat::Bmpd,
        )
        .unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 2);
        assert_eq!(doctor.state.malformed_messages, 0);
        assert_eq!(*doctor.state.by_type.get(&4).unwrap_or(&0), 1); // Initiation
        assert_eq!(*doctor.state.by_type.get(&5).unwrap_or(&0), 1); // Termination

        // Initiation TLVs
        let init = doctor.state.initiation_info.as_ref().unwrap();
        assert_eq!(init.strings.len(), 2);
        assert_eq!(init.strings[0].value, "FRRouting");
        assert_eq!(init.strings[1].value, "bmp-speaker");

        // Termination reason
        let term = doctor.state.termination_info.as_ref().unwrap();
        assert_eq!(term.termination_reason, Some(2));
        assert!(crate::raw_bmp::termination_reason_name(2).contains("closed"));
    }

    #[test]
    fn test_init_term_tlv_rawbmp_fixture() {
        // Read-only regression test against raw BMP fixture (no container)
        let mut doctor = Doctor::with_max_findings(
            Path::new("tests/fixtures/init-term-tlvs.rawbmp"),
            1000,
            InputFormat::RawBmp,
        )
        .unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 2);
        assert_eq!(doctor.state.malformed_messages, 0);

        let init = doctor.state.initiation_info.as_ref().unwrap();
        assert_eq!(init.strings[0].value, "FRRouting");
        assert_eq!(init.strings[1].value, "bmp-speaker");

        let term = doctor.state.termination_info.as_ref().unwrap();
        assert_eq!(term.termination_reason, Some(2));
    }

    #[test]
    fn test_peer_up_down_rawbmp_fixture() {
        // Read-only regression test: clean Peer Up -> Peer Down lifecycle.
        // The synthetic BGP OPEN may produce a bgpkit-parser parse_error;
        // that is expected and is not a BMP framing or lifecycle failure.
        let mut doctor = Doctor::with_max_findings(
            Path::new("tests/fixtures/peer-up-down.rawbmp"),
            1000,
            InputFormat::RawBmp,
        )
        .unwrap();
        doctor.process(false).unwrap();

        assert_eq!(doctor.state.total_messages, 2);
        assert_eq!(doctor.state.malformed_messages, 0);
        assert_eq!(doctor.state.peers.len(), 1);

        let peer = doctor.state.peers.values().next().unwrap();
        assert_eq!(peer.peer_up_count, 1);
        assert_eq!(peer.peer_down_count, 1);
        assert!(!peer.active);
        assert!(peer.peer_up_seen);
        assert_eq!(peer.last_peer_down_reason, Some(2));

        // No lifecycle warnings in a clean session
        let has_down_without_up = doctor
            .state
            .findings
            .iter()
            .any(|f| f.rule == "peer_down_without_peer_up");
        assert!(!has_down_without_up);
    }
}
