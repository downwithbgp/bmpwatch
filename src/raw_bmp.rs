use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use crate::error::DoctorError;

// BMP Common Header as defined in RFC 7854, Section 3.1.
// Layout: Version (1) + Message Length (4) + Message Type (1) = 6 bytes.
pub const BMP_COMMON_HEADER_SIZE: usize = 6;

// BMP Per-Peer Header as defined in RFC 7854, Section 3.2.
// Layout: Peer Type (1) + Peer Flags (1) + Peer Distinguisher (8)
//       + Peer Address (16) + Peer AS (4) + Peer BGP ID (4)
//       + Timestamp (8) = 42 bytes.
pub const BMP_PER_PEER_HEADER_SIZE: usize = 42;

// RFC 7854, Section 9: the only defined BMP version is 3.
pub const BMP_EXPECTED_VERSION: u8 = 3;

// BMP Message Types as registered in IANA BMP Parameters and defined in
// RFC 7854 Sections 3.3–3.8 and Section 8 (Route Mirroring).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BmpMessageType {
    RouteMonitoring = 0,       // RFC 7854, Section 3.3
    StatisticsReport = 1,      // RFC 7854, Section 3.4
    PeerDownNotification = 2,  // RFC 7854, Section 3.5
    PeerUpNotification = 3,    // RFC 7854, Section 3.6
    InitiationMessage = 4,     // RFC 7854, Section 3.7
    TerminationMessage = 5,    // RFC 7854, Section 3.8
    RouteMirroringMessage = 6, // RFC 7854, Section 8
}

impl BmpMessageType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(BmpMessageType::RouteMonitoring),
            1 => Some(BmpMessageType::StatisticsReport),
            2 => Some(BmpMessageType::PeerDownNotification),
            3 => Some(BmpMessageType::PeerUpNotification),
            4 => Some(BmpMessageType::InitiationMessage),
            5 => Some(BmpMessageType::TerminationMessage),
            6 => Some(BmpMessageType::RouteMirroringMessage),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            BmpMessageType::RouteMonitoring => "RouteMonitoring",
            BmpMessageType::StatisticsReport => "StatisticsReport",
            BmpMessageType::PeerDownNotification => "PeerDownNotification",
            BmpMessageType::PeerUpNotification => "PeerUpNotification",
            BmpMessageType::InitiationMessage => "InitiationMessage",
            BmpMessageType::TerminationMessage => "TerminationMessage",
            BmpMessageType::RouteMirroringMessage => "RouteMirroringMessage",
        }
    }

    // RFC 7854, Section 4.2: the Per-Peer Header is present in Route Monitoring
    // (0), Statistics Report (1), Peer Down (2), Peer Up (3), and Route
    // Mirroring (6) messages. Initiation (4) and Termination (5) do not have one.
    pub fn has_per_peer_header(self) -> bool {
        matches!(
            self,
            BmpMessageType::RouteMonitoring
                | BmpMessageType::StatisticsReport
                | BmpMessageType::PeerDownNotification
                | BmpMessageType::PeerUpNotification
                | BmpMessageType::RouteMirroringMessage
        )
    }
}

// Per-Peer Header fields as defined in RFC 7854, Section 3.2.
// Total size: 42 bytes (BMP_PER_PEER_HEADER_SIZE).
#[derive(Debug, Clone)]
pub struct PerPeerHeader {
    // Byte 0: Peer Type (RFC 7854, Section 3.2). 0 = Global Instance,
    // 1 = RD Instance, 2 = Local Instance.
    pub peer_type: u8,
    // Byte 1: Peer Flags (RFC 7854, Section 3.2).
    // Bit 7 (0x80): V flag — if set, Peer Address is IPv6, otherwise IPv4.
    // Bit 6 (0x40): L flag — post-policy Adj-RIB-Out (RFC 8671).
    // Bit 5 (0x20): A flag — AS_PATH attribute includes 2 preceding ASNs (legacy).
    pub peer_flags: u8,
    // Bytes 2–9: Peer Distinguisher (RFC 7854, Section 3.2). Zero-filled
    // for Global Instance peers.
    pub peer_distinguisher: [u8; 8],
    // Bytes 10–25: Peer Address (RFC 7854, Section 3.2).
    // IPv4 addresses use IPv4-mapped IPv6 format (::ffff:x.x.x.x)
    // at bytes 12–15 of this field.
    pub peer_address: [u8; 16],
    // Bytes 26–29: Peer AS (RFC 7854, Section 3.2). 4-byte ASN in network byte order.
    pub peer_asn: u32,
    // Bytes 30–33: Peer BGP ID (RFC 7854, Section 3.2). Router ID of the peer.
    #[allow(dead_code)]
    pub peer_bgp_id: [u8; 4],
    // Bytes 34–41: Timestamp (RFC 7854, Section 3.2).
    // Split into seconds (bytes 34–37) and microseconds (bytes 38–41).
    pub timestamp_seconds: u32,
    pub timestamp_microseconds: u32,
}

impl PerPeerHeader {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < BMP_PER_PEER_HEADER_SIZE {
            return None;
        }
        let peer_type = data[0];
        let peer_flags = data[1];
        let mut peer_distinguisher = [0u8; 8];
        peer_distinguisher.copy_from_slice(&data[2..10]);
        let mut peer_address = [0u8; 16];
        peer_address.copy_from_slice(&data[10..26]);
        let peer_asn = u32::from_be_bytes([data[26], data[27], data[28], data[29]]);
        let mut peer_bgp_id = [0u8; 4];
        peer_bgp_id.copy_from_slice(&data[30..34]);
        let timestamp_seconds = u32::from_be_bytes([data[34], data[35], data[36], data[37]]);
        let timestamp_microseconds = u32::from_be_bytes([data[38], data[39], data[40], data[41]]);

        Some(PerPeerHeader {
            peer_type,
            peer_flags,
            peer_distinguisher,
            peer_address,
            peer_asn,
            peer_bgp_id,
            timestamp_seconds,
            timestamp_microseconds,
        })
    }

    pub fn peer_ip(&self) -> String {
        if self.is_ipv6() {
            format!(
                "{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}",
                self.peer_address[0], self.peer_address[1],
                self.peer_address[2], self.peer_address[3],
                self.peer_address[4], self.peer_address[5],
                self.peer_address[6], self.peer_address[7],
                self.peer_address[8], self.peer_address[9],
                self.peer_address[10], self.peer_address[11],
                self.peer_address[12], self.peer_address[13],
                self.peer_address[14], self.peer_address[15],
            )
        } else {
            format!(
                "{}.{}.{}.{}",
                self.peer_address[12],
                self.peer_address[13],
                self.peer_address[14],
                self.peer_address[15],
            )
        }
    }

    /// RFC 7854, Section 3.2: the V flag (bit 7, 0x80) in Peer Flags indicates
    /// whether the Peer Address field carries an IPv6 address (V=1) or an
    /// IPv4-mapped IPv6 address (V=0).
    fn is_ipv6(&self) -> bool {
        (self.peer_flags & 0x80) != 0
    }
}

#[derive(Debug, Clone)]
pub struct RawBmpFrame {
    pub offset: u64,
    pub version: u8,
    pub msg_len: u32,
    pub msg_type_raw: u8,
    pub msg_type: Option<BmpMessageType>,
    pub per_peer_header: Option<PerPeerHeader>,
    pub payload: Vec<u8>,
    pub full_data: Vec<u8>,
    pub tlv_info: Option<TlvInfo>,
    /// Stats Report entries, if message type is StatisticsReport (1).
    pub stats_info: Option<StatsInfo>,
    /// Peer Down reason code, if message type is PeerDownNotification (2).
    pub peer_down_info: Option<PeerDownInfo>,
}

/// Peer Down Notification reason (RFC 7854, Section 3.5).
#[derive(Debug, Clone, serde::Serialize)]
pub struct PeerDownInfo {
    pub reason_code: u8,
    pub reason_name: String,
}

/// Stats Report information (RFC 7854, Section 3.4).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StatsInfo {
    pub entries: Vec<StatsEntry>,
}

/// A single statistic entry from a Stats Report message.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatsEntry {
    pub stat_type: u16,
    pub stat_name: String,
    pub stat_value: u64,
}

/// Known Stats Report stat type names (RFC 7854 + IANA BMP Parameters).
fn stat_type_name(t: u16) -> &'static str {
    match t {
        0 => "Prefixes rejected by inbound policy",
        1 => "Duplicate prefix advertisements received",
        2 => "Duplicate withdraws received",
        3 => "Updates invalidated due to CLUSTER_LIST loop",
        4 => "Updates invalidated due to AS_PATH loop detection",
        5 => "Updates invalidated due to ORIGINATOR_ID",
        6 => "Updates invalidated due to AS_CONFED loop detection",
        7 => "Routes in Adj-RIBs-In",
        8 => "Routes in Loc-RIB",
        9 => "Routes in per-AFI/SAFI Adj-RIB-In",
        10 => "Routes in per-AFI/SAFI Loc-RIB",
        11 => "Updates subjected to AS_PATH update treatment",
        12 => "Prefixes subjected to AS_PATH update treatment",
        13 => "Duplicate updates",
        _ => "Unknown",
    }
}

/// Parse Stats Report payload after the Per-Peer Header.
/// RFC 7854, Section 3.4: Stats Count (4) + repeated entries of Stat Type (2) + Stat Len (2) + value.
fn parse_stats(payload: &[u8]) -> Option<StatsInfo> {
    if payload.len() < 4 {
        return None;
    }
    let stats_count = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    let mut entries = Vec::new();
    let mut pos = 4;

    for _ in 0..stats_count {
        if pos + 4 > payload.len() {
            break;
        }
        let stat_type = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
        let stat_len = u16::from_be_bytes([payload[pos + 2], payload[pos + 3]]) as usize;
        pos += 4;

        if pos + stat_len > payload.len() {
            break;
        }
        let data = &payload[pos..pos + stat_len];
        pos += stat_len;

        let value = match data.len() {
            4 => u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as u64,
            8 => u64::from_be_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]),
            _ => data.len() as u64,
        };
        entries.push(StatsEntry {
            stat_type,
            stat_name: stat_type_name(stat_type).to_string(),
            stat_value: value,
        });
    }

    if entries.is_empty() {
        None
    } else {
        Some(StatsInfo { entries })
    }
}

/// BMP Information TLV entry (RFC 7854, Sections 3.7–3.8).
/// Used by Initiation and Termination messages.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TlvInfo {
    /// Known information strings (sysDescr, sysName, etc.).
    pub strings: Vec<TlvString>,
    /// Termination reason code, if present (type=1, 2-byte value).
    pub termination_reason: Option<u16>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TlvString {
    pub tlv_type: u16,
    pub type_name: String,
    pub value: String,
}

/// Known Initiation TLV type names (RFC 7854 + IANA BMP Parameters).
fn init_tlv_name(t: u16) -> &'static str {
    match t {
        0 => "String",
        1 => "sysDescr",
        2 => "sysName",
        _ => "Unknown",
    }
}

/// Known BMP Peer Down reason code names (RFC 7854 Section 3.5 + IANA BMP Parameters).
/// This is separate from Termination reason codes — they use different registries.
pub fn peer_down_reason_name(code: u8) -> &'static str {
    match code {
        0 => "Reserved",
        1 => "Local system closed, NOTIFICATION sent",
        2 => "Local system closed, no NOTIFICATION",
        3 => "Remote system closed, NOTIFICATION received",
        4 => "Remote system closed, no data",
        5 => "Peer de-configured",
        6 => "Local system closed, session termination (NOTIFICATION sent)",
        7 => "Local system closed, session termination (no NOTIFICATION)",
        _ => "Unknown",
    }
}

/// Known BMP Termination reason code names (RFC 7854 Section 3.8 + IANA BMP Parameters).
pub fn termination_reason_name(code: u16) -> &'static str {
    match code {
        0 => "Reserved",
        1 => "Administratively prohibited",
        2 => "Administratively closed",
        3 => "Unspecified",
        4 => "Out of resources",
        5 => "Redundant connection",
        6 => "Permanently administratively prohibited",
        _ => "Unknown",
    }
}

/// Parse Information TLVs from Initiation or Termination message payload.
/// RFC 7854, Section 3.7: TLV format is Type(2) + Length(2) + Value(variable).
fn parse_tlvs(payload: &[u8], msg_type: BmpMessageType) -> Option<TlvInfo> {
    if payload.len() < 4 {
        return None;
    }

    let mut strings = Vec::new();
    let mut termination_reason = None;
    let mut pos = 0;

    while pos + 4 <= payload.len() {
        let tlv_type = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
        let tlv_len = u16::from_be_bytes([payload[pos + 2], payload[pos + 3]]) as usize;
        pos += 4;

        if pos + tlv_len > payload.len() {
            // Truncated TLV — stop parsing, what we have is valid
            break;
        }

        let value = &payload[pos..pos + tlv_len];
        pos += tlv_len;

        match msg_type {
            BmpMessageType::InitiationMessage => {
                let type_name = init_tlv_name(tlv_type).to_string();
                if let Ok(s) = std::str::from_utf8(value) {
                    strings.push(TlvString {
                        tlv_type,
                        type_name,
                        value: s.trim_end_matches('\0').to_string(),
                    });
                } else {
                    strings.push(TlvString {
                        tlv_type,
                        type_name,
                        value: format!("(non-UTF8, {tlv_len} bytes)"),
                    });
                }
            }
            BmpMessageType::TerminationMessage => {
                if tlv_type == 1 && value.len() >= 2 {
                    termination_reason = Some(u16::from_be_bytes([value[0], value[1]]));
                } else if let Ok(s) = std::str::from_utf8(value) {
                    strings.push(TlvString {
                        tlv_type,
                        type_name: if tlv_type == 0 { "String" } else { "Unknown" }.to_string(),
                        value: s.trim_end_matches('\0').to_string(),
                    });
                }
            }
            _ => {}
        }
    }

    if strings.is_empty() && termination_reason.is_none() {
        None
    } else {
        Some(TlvInfo {
            strings,
            termination_reason,
        })
    }
}

/// Parse a single raw BMP frame from a byte slice.
/// `file_offset` is the byte offset within the source file where this frame begins.
pub fn parse_frame_from_bytes(data: &[u8], file_offset: u64) -> Result<RawBmpFrame, DoctorError> {
    if data.len() < BMP_COMMON_HEADER_SIZE {
        return Err(DoctorError::Frame(format!(
            "Incomplete BMP common header at offset {file_offset}: need {BMP_COMMON_HEADER_SIZE} bytes, have {}",
            data.len()
        )));
    }

    let version = data[0];
    let msg_len = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
    let msg_type_raw = data[5];
    let msg_type = BmpMessageType::from_u8(msg_type_raw);

    if msg_len < BMP_COMMON_HEADER_SIZE as u32 {
        return Err(DoctorError::Frame(format!(
            "Invalid BMP message length {msg_len} at offset {file_offset}: must be >= {BMP_COMMON_HEADER_SIZE}"
        )));
    }

    let total = msg_len as usize;
    if data.len() < total {
        return Err(DoctorError::Frame(format!(
            "Truncated BMP frame at offset {file_offset}: declared length {msg_len} exceeds available {} bytes",
            data.len()
        )));
    }

    let payload = &data[BMP_COMMON_HEADER_SIZE..total];

    let per_peer_header = if let Some(mt) = msg_type {
        if mt.has_per_peer_header() {
            PerPeerHeader::parse(payload)
        } else {
            None
        }
    } else {
        None
    };

    let tlv_info = match msg_type {
        Some(BmpMessageType::InitiationMessage | BmpMessageType::TerminationMessage) => {
            parse_tlvs(payload, msg_type.unwrap())
        }
        _ => None,
    };

    let stats_info = match msg_type {
        Some(BmpMessageType::StatisticsReport) => payload
            .get(BMP_PER_PEER_HEADER_SIZE..)
            .and_then(parse_stats),
        _ => None,
    };

    let peer_down_info =
        match msg_type {
            Some(BmpMessageType::PeerDownNotification) => payload
                .get(BMP_PER_PEER_HEADER_SIZE)
                .map(|&code| PeerDownInfo {
                    reason_code: code,
                    reason_name: peer_down_reason_name(code).to_string(),
                }),
            _ => None,
        };

    let mut full_data = Vec::with_capacity(total);
    full_data.extend_from_slice(&data[..total]);

    Ok(RawBmpFrame {
        offset: file_offset,
        version,
        msg_len,
        msg_type_raw,
        msg_type,
        per_peer_header,
        payload: payload.to_vec(),
        full_data,
        tlv_info,
        stats_info,
        peer_down_info,
    })
}

pub struct RawBmpIterator {
    reader: BufReader<File>,
    offset: u64,
}

impl RawBmpIterator {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, DoctorError> {
        let file = File::open(&path)?;
        Ok(RawBmpIterator {
            reader: BufReader::new(file),
            offset: 0,
        })
    }
}

impl Iterator for RawBmpIterator {
    type Item = Result<RawBmpFrame, DoctorError>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut header_buf = [0u8; BMP_COMMON_HEADER_SIZE];
        match self.reader.read_exact(&mut header_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                if self.offset == 0 {
                    self.offset = 1; // prevent infinite loop on empty/truncated files
                    return Some(Err(DoctorError::Frame(
                        "File appears empty or truncated".to_string(),
                    )));
                }
                return None;
            }
            Err(e) => return Some(Err(DoctorError::Io(e))),
        }

        let frame_offset = self.offset;
        self.offset += BMP_COMMON_HEADER_SIZE as u64;

        let version = header_buf[0];
        let msg_len =
            u32::from_be_bytes([header_buf[1], header_buf[2], header_buf[3], header_buf[4]]);
        let msg_type_raw = header_buf[5];
        let msg_type = BmpMessageType::from_u8(msg_type_raw);

        if msg_len < BMP_COMMON_HEADER_SIZE as u32 {
            return Some(Err(DoctorError::Frame(format!(
                "Invalid BMP message length {msg_len} at offset {frame_offset}: must be >= {BMP_COMMON_HEADER_SIZE}"
            ))));
        }

        let payload_len = (msg_len as usize).saturating_sub(BMP_COMMON_HEADER_SIZE);
        let mut payload = vec![0u8; payload_len];
        if let Err(e) = self.reader.read_exact(&mut payload) {
            return Some(Err(DoctorError::Frame(format!(
                "Truncated frame at offset {frame_offset}: declared length {msg_len} exceeds available data: {e}"
            ))));
        }
        self.offset += payload_len as u64;

        let per_peer_header = if let Some(mt) = msg_type {
            if mt.has_per_peer_header() {
                PerPeerHeader::parse(&payload)
            } else {
                None
            }
        } else {
            None
        };

        let mut full_data = Vec::with_capacity(BMP_COMMON_HEADER_SIZE + payload.len());
        full_data.extend_from_slice(&header_buf);
        full_data.extend_from_slice(&payload);

        let tlv_info = match msg_type {
            Some(BmpMessageType::InitiationMessage | BmpMessageType::TerminationMessage) => {
                parse_tlvs(&payload, msg_type.unwrap())
            }
            _ => None,
        };

        let stats_info = match msg_type {
            Some(BmpMessageType::StatisticsReport) => payload
                .get(BMP_PER_PEER_HEADER_SIZE..)
                .and_then(parse_stats),
            _ => None,
        };

        let peer_down_info = match msg_type {
            Some(BmpMessageType::PeerDownNotification) => payload
                .get(BMP_PER_PEER_HEADER_SIZE)
                .map(|&code| PeerDownInfo {
                    reason_code: code,
                    reason_name: peer_down_reason_name(code).to_string(),
                }),
            _ => None,
        };

        Some(Ok(RawBmpFrame {
            offset: frame_offset,
            version,
            msg_len,
            msg_type_raw,
            msg_type,
            per_peer_header,
            payload,
            full_data,
            tlv_info,
            stats_info,
            peer_down_info,
        }))
    }
}

#[cfg(test)]
pub mod fixtures {
    use crate::raw_bmp::{BMP_COMMON_HEADER_SIZE, BMP_EXPECTED_VERSION, BMP_PER_PEER_HEADER_SIZE};

    pub fn make_common_header(msg_type: u8, payload_len: u32) -> Vec<u8> {
        let msg_len = BMP_COMMON_HEADER_SIZE as u32 + payload_len;
        let mut buf = Vec::with_capacity(BMP_COMMON_HEADER_SIZE);
        buf.push(BMP_EXPECTED_VERSION);
        buf.extend_from_slice(&msg_len.to_be_bytes());
        buf.push(msg_type);
        buf
    }

    pub fn make_per_peer_header(
        peer_asn: u32,
        ip_octets: [u8; 4],
        ts_sec: u32,
        ts_us: u32,
    ) -> Vec<u8> {
        let mut buf = vec![0u8; BMP_PER_PEER_HEADER_SIZE];
        buf[0] = 0;
        buf[1] = 0;
        // peer_distinguisher: all zeros (bytes 2-9), already zeroed
        // peer_address: IPv4-mapped IPv6 (bytes 10-25)
        //   bytes 10-19: 10 zero bytes (already zeroed)
        //   bytes 20-21: 0xFF 0xFF (IPv4-mapped prefix)
        //   bytes 22-25: IPv4 address
        buf[20] = 0xff;
        buf[21] = 0xff;
        buf[22..26].copy_from_slice(&ip_octets);
        // peer_asn (bytes 26-29)
        buf[26..30].copy_from_slice(&peer_asn.to_be_bytes());
        // peer_bgp_id: zeros (bytes 30-33), already zeroed
        // timestamp (bytes 34-41)
        buf[34..38].copy_from_slice(&ts_sec.to_be_bytes());
        buf[38..42].copy_from_slice(&ts_us.to_be_bytes());
        buf
    }

    fn make_minimal_open_message(local_as: u32, ip_octets: [u8; 4]) -> Vec<u8> {
        let mut open = vec![0x04];
        open.extend_from_slice(&(local_as as u16).to_be_bytes());
        open.extend_from_slice(&0xB4u16.to_be_bytes());
        open.extend_from_slice(&ip_octets);
        open.push(0x00);
        open
    }

    pub fn make_peer_up_frame(
        peer_asn: u32,
        ip_octets: [u8; 4],
        ts_sec: u32,
        ts_us: u32,
    ) -> Vec<u8> {
        let sent_open = make_minimal_open_message(peer_asn, ip_octets);
        let local_addr = [0u8; 16];
        let local_port = 0u16;
        let remote_port = 179u16;

        let mut payload = Vec::new();
        payload.extend_from_slice(&local_addr);
        payload.extend_from_slice(&local_port.to_be_bytes());
        payload.extend_from_slice(&remote_port.to_be_bytes());
        payload.extend_from_slice(&sent_open);

        let pph = make_per_peer_header(peer_asn, ip_octets, ts_sec, ts_us);
        let total_payload = [pph.as_slice(), payload.as_slice()].concat();
        let header = make_common_header(3, total_payload.len() as u32);

        let mut frame = Vec::new();
        frame.extend_from_slice(&header);
        frame.extend_from_slice(&total_payload);
        frame
    }

    pub fn make_peer_down_frame(
        peer_asn: u32,
        ip_octets: [u8; 4],
        ts_sec: u32,
        ts_us: u32,
        reason: u8,
    ) -> Vec<u8> {
        let reason_payload = vec![reason];
        let pph = make_per_peer_header(peer_asn, ip_octets, ts_sec, ts_us);
        let total_payload = [pph.as_slice(), reason_payload.as_slice()].concat();
        let header = make_common_header(2, total_payload.len() as u32);

        let mut frame = Vec::new();
        frame.extend_from_slice(&header);
        frame.extend_from_slice(&total_payload);
        frame
    }

    pub fn make_route_monitoring_frame(
        peer_asn: u32,
        ip_octets: [u8; 4],
        ts_sec: u32,
        ts_us: u32,
    ) -> Vec<u8> {
        let bgp_update: Vec<u8> = vec![0x00, 0x00, 0x00, 0x00];
        let pph = make_per_peer_header(peer_asn, ip_octets, ts_sec, ts_us);
        let total_payload = [pph.as_slice(), bgp_update.as_slice()].concat();
        let header = make_common_header(0, total_payload.len() as u32);

        let mut frame = Vec::new();
        frame.extend_from_slice(&header);
        frame.extend_from_slice(&total_payload);
        frame
    }

    pub fn make_initiation_frame(sys_name: &str) -> Vec<u8> {
        let tlv_type = [0x00, 0x00];
        let name_bytes = sys_name.as_bytes();
        let tlv_len = (name_bytes.len() as u16).to_be_bytes();
        let mut payload = Vec::new();
        payload.extend_from_slice(&tlv_type);
        payload.extend_from_slice(&tlv_len);
        payload.extend_from_slice(name_bytes);

        let header = make_common_header(4, payload.len() as u32);
        let mut frame = Vec::new();
        frame.extend_from_slice(&header);
        frame.extend_from_slice(&payload);
        frame
    }

    pub fn make_invalid_version_frame() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(0xFF);
        buf.extend_from_slice(&10u32.to_be_bytes());
        buf.push(0);
        buf.extend_from_slice(&[0u8; 4]);
        buf
    }

    pub fn make_truncated_frame() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(BMP_EXPECTED_VERSION);
        buf.extend_from_slice(&1000u32.to_be_bytes());
        buf.push(0);
        buf.extend_from_slice(&[0u8; 4]);
        buf
    }

    pub fn make_invalid_length_frame() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(BMP_EXPECTED_VERSION);
        buf.extend_from_slice(&4u32.to_be_bytes());
        buf.push(0);
        buf
    }

    pub fn write_fixture(frames: &[Vec<u8>]) -> Vec<u8> {
        let mut data = Vec::new();
        for frame in frames {
            data.extend_from_slice(frame);
        }
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_common_header_generation() {
        let header = fixtures::make_common_header(0, 10);
        assert_eq!(header.len(), BMP_COMMON_HEADER_SIZE);
        assert_eq!(header[0], BMP_EXPECTED_VERSION);
        assert_eq!(
            u32::from_be_bytes([header[1], header[2], header[3], header[4]]),
            16
        );
        assert_eq!(header[5], 0);
    }

    #[test]
    fn test_per_peer_header_parsing() {
        let pph_bytes = fixtures::make_per_peer_header(65000, [10, 0, 0, 1], 1000, 500);
        assert_eq!(pph_bytes.len(), BMP_PER_PEER_HEADER_SIZE);

        let pph = PerPeerHeader::parse(&pph_bytes).expect("Should parse per-peer header");
        assert_eq!(pph.peer_asn, 65000);
        assert_eq!(pph.timestamp_seconds, 1000);
        assert_eq!(pph.timestamp_microseconds, 500);
        assert_eq!(pph.peer_ip(), "10.0.0.1");
    }

    #[test]
    fn test_raw_bmp_iterator_valid_frames() {
        let peer_up = fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let rm = fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 200, 0);
        let peer_down = fixtures::make_peer_down_frame(65000, [10, 0, 0, 1], 300, 0, 3);
        let data = fixtures::write_fixture(&[peer_up, rm, peer_down]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let iter = RawBmpIterator::open(&path).unwrap();
        let frames: Vec<_> = iter.collect::<Result<Vec<_>, _>>().unwrap();

        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].msg_type, Some(BmpMessageType::PeerUpNotification));
        assert_eq!(frames[1].msg_type, Some(BmpMessageType::RouteMonitoring));
        assert_eq!(
            frames[2].msg_type,
            Some(BmpMessageType::PeerDownNotification)
        );
    }

    #[test]
    fn test_raw_bmp_iterator_invalid_version() {
        let bad = fixtures::make_invalid_version_frame();

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&bad).unwrap();
        let path = tmp.into_temp_path();

        let iter = RawBmpIterator::open(&path).unwrap();
        let frames: Vec<_> = iter.collect();
        assert_eq!(frames.len(), 1);
        let frame = frames[0].as_ref().unwrap();
        assert_eq!(frame.version, 0xFF);
        assert_eq!(frame.msg_type, Some(BmpMessageType::RouteMonitoring));
    }

    #[test]
    fn test_raw_bmp_iterator_truncated_frame() {
        let bad = fixtures::make_truncated_frame();
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&bad).unwrap();
        let path = tmp.into_temp_path();

        let iter = RawBmpIterator::open(&path).unwrap();
        let frames: Vec<_> = iter.collect();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].is_err());
    }

    #[test]
    fn test_raw_bmp_iterator_invalid_length() {
        let bad = fixtures::make_invalid_length_frame();

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&bad).unwrap();
        let path = tmp.into_temp_path();

        let iter = RawBmpIterator::open(&path).unwrap();
        let frames: Vec<_> = iter.collect();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].is_err());
    }

    #[test]
    fn test_raw_bmp_iterator_initiation() {
        let init = fixtures::make_initiation_frame("test-router");

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&init).unwrap();
        let path = tmp.into_temp_path();

        let iter = RawBmpIterator::open(&path).unwrap();
        let frames: Vec<_> = iter.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].msg_type, Some(BmpMessageType::InitiationMessage));
        assert!(frames[0].per_peer_header.is_none());
    }

    #[test]
    fn test_parse_initiation_tlvs() {
        // Build Initiation message with sysDescr (type=1) and sysName (type=2)
        let sys_descr = b"FRRouting 8.5.1";
        let sys_name = b"bmp-speaker";

        let tlv1 = make_tlv(1, sys_descr);
        let tlv2 = make_tlv(2, sys_name);
        let payload = [tlv1, tlv2].concat();

        let frame = make_bmp_message(4, &payload);
        let frame = parse_frame_from_bytes(&frame, 0).unwrap();
        assert_eq!(frame.msg_type, Some(BmpMessageType::InitiationMessage));
        let tlv = frame.tlv_info.as_ref().unwrap();
        assert_eq!(tlv.strings.len(), 2);
        assert_eq!(tlv.strings[0].tlv_type, 1);
        assert_eq!(tlv.strings[0].value, "FRRouting 8.5.1");
        assert_eq!(tlv.strings[1].tlv_type, 2);
        assert_eq!(tlv.strings[1].value, "bmp-speaker");
    }

    #[test]
    fn test_parse_termination_tlv_reason() {
        // Build Termination message with reason TLV (type=1, 2-byte value)
        let reason: u16 = 2; // Administrative shutdown
        let tlv = make_tlv(1, &reason.to_be_bytes());
        let payload = tlv;
        let frame = make_bmp_message(5, &payload);
        let frame = parse_frame_from_bytes(&frame, 0).unwrap();
        let tlv = frame.tlv_info.as_ref().unwrap();
        assert_eq!(tlv.termination_reason, Some(2));
        assert!(tlv.strings.is_empty());
    }

    #[test]
    fn test_termination_reason_names() {
        assert_eq!(termination_reason_name(1), "Administratively prohibited");
        assert_eq!(termination_reason_name(2), "Administratively closed");
        assert_eq!(termination_reason_name(999), "Unknown");
    }

    #[test]
    fn test_peer_down_reason_names() {
        // RFC 7854 Peer Down codes (separate registry from Termination)
        assert_eq!(
            peer_down_reason_name(1),
            "Local system closed, NOTIFICATION sent"
        );
        assert_eq!(
            peer_down_reason_name(2),
            "Local system closed, no NOTIFICATION"
        );
        assert_eq!(peer_down_reason_name(5), "Peer de-configured");
        assert_eq!(peer_down_reason_name(255), "Unknown");
    }

    #[test]
    fn test_parse_tlvs_truncated() {
        // TLV with declared length exceeding available data
        let mut payload = vec![0u8; 8];
        payload[0..2].copy_from_slice(&1u16.to_be_bytes()); // type=sysDescr
        payload[2..4].copy_from_slice(&100u16.to_be_bytes()); // len=100, only 4 bytes available
        let frame = make_bmp_message(4, &payload);
        // Should not panic, just return None for tlv_info
        let frame = parse_frame_from_bytes(&frame, 0).unwrap();
        assert!(frame.tlv_info.is_none());
    }

    #[test]
    fn test_parse_tlvs_unknown_type() {
        let tlv = make_tlv(99, b"unknown data");
        let payload = tlv;
        let frame = make_bmp_message(4, &payload);
        let frame = parse_frame_from_bytes(&frame, 0).unwrap();
        let tlv = frame.tlv_info.as_ref().unwrap();
        assert_eq!(tlv.strings.len(), 1);
        assert_eq!(tlv.strings[0].type_name, "Unknown");
        assert_eq!(tlv.strings[0].tlv_type, 99);
    }

    fn make_tlv(t: u16, value: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&t.to_be_bytes());
        buf.extend_from_slice(&(value.len() as u16).to_be_bytes());
        buf.extend_from_slice(value);
        buf
    }

    fn make_bmp_message(msg_type: u8, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(3); // version
        let msg_len = (6 + payload.len()) as u32;
        buf.extend_from_slice(&msg_len.to_be_bytes());
        buf.push(msg_type);
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn test_parse_stats_report() {
        // Build Stats Report with per-peer header + 2 stat entries
        let pph = fixtures::make_per_peer_header(65000, [10, 0, 0, 1], 100, 0);
        let mut payload = vec![0u8; 4]; // stats_count
        payload[0..4].copy_from_slice(&2u32.to_be_bytes());
        // Entry 1: type 7 (Adj-RIBs-In routes), len=4, value=42
        let e1 = make_stats_entry(7, &42u32.to_be_bytes());
        payload.extend_from_slice(&e1);
        // Entry 2: type 8 (Loc-RIB routes), len=4, value=10
        let e2 = make_stats_entry(8, &10u32.to_be_bytes());
        payload.extend_from_slice(&e2);
        let total = [pph.as_slice(), payload.as_slice()].concat();
        let frame = make_bmp_message(1, &total);
        let frame = parse_frame_from_bytes(&frame, 0).unwrap();
        let stats = frame.stats_info.as_ref().unwrap();
        assert_eq!(stats.entries.len(), 2);
        assert_eq!(stats.entries[0].stat_type, 7);
        assert_eq!(stats.entries[0].stat_value, 42);
        assert!(stats.entries[0].stat_name.contains("Adj-RIBs-In"));
        assert_eq!(stats.entries[1].stat_type, 8);
        assert_eq!(stats.entries[1].stat_value, 10);
    }

    #[test]
    fn test_parse_stats_unknown_type() {
        let pph = fixtures::make_per_peer_header(65000, [10, 0, 0, 1], 100, 0);
        let payload = [
            1u32.to_be_bytes().as_slice(),
            make_stats_entry(99, &1u32.to_be_bytes()).as_slice(),
        ]
        .concat();
        let total = [pph.as_slice(), payload.as_slice()].concat();
        let frame = make_bmp_message(1, &total);
        let stats = parse_frame_from_bytes(&frame, 0)
            .unwrap()
            .stats_info
            .unwrap();
        assert_eq!(stats.entries[0].stat_name, "Unknown");
    }

    #[test]
    fn test_parse_stats_truncated() {
        let pph = fixtures::make_per_peer_header(65000, [10, 0, 0, 1], 100, 0);
        // Declare 99 entries but only provide 1
        let mut payload = vec![0u8; 4];
        payload[0..4].copy_from_slice(&99u32.to_be_bytes());
        payload.extend_from_slice(&make_stats_entry(7, &42u32.to_be_bytes()));
        let total = [pph.as_slice(), payload.as_slice()].concat();
        let frame = make_bmp_message(1, &total);
        let stats = parse_frame_from_bytes(&frame, 0)
            .unwrap()
            .stats_info
            .unwrap();
        assert_eq!(stats.entries.len(), 1); // stops at truncation
    }

    fn make_stats_entry(t: u16, value: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&t.to_be_bytes());
        buf.extend_from_slice(&(value.len() as u16).to_be_bytes());
        buf.extend_from_slice(value);
        buf
    }

    #[test]
    fn test_short_statistics_report_no_panic() {
        // Crafted 8-byte frame that would panic before C1 fix:
        // version=3, msg_len=8, type=1 (StatisticsReport), 2 payload bytes.
        // payload.len() = 2 < BMP_PER_PEER_HEADER_SIZE (42), so slicing panics.
        let frame: [u8; 8] = [0x03, 0x00, 0x00, 0x00, 0x08, 0x01, 0xFF, 0xFF];
        let result = parse_frame_from_bytes(&frame, 0).unwrap();
        assert_eq!(result.msg_type, Some(BmpMessageType::StatisticsReport));
        // stats_info should be None, not panic
        assert!(result.stats_info.is_none());
    }

    #[test]
    fn test_short_statistics_report_iterator_no_panic() {
        // Same crafted 8-byte frame, but exercised through RawBmpIterator.
        let frame: [u8; 8] = [0x03, 0x00, 0x00, 0x00, 0x08, 0x01, 0xFF, 0xFF];

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&frame).unwrap();
        let path = tmp.into_temp_path();

        let iter = RawBmpIterator::open(&path).unwrap();
        let frames: Vec<_> = iter.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].msg_type, Some(BmpMessageType::StatisticsReport));
        assert!(frames[0].stats_info.is_none());
    }
}
