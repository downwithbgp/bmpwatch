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

        Some(Ok(RawBmpFrame {
            offset: frame_offset,
            version,
            msg_len,
            msg_type_raw,
            msg_type,
            per_peer_header,
            payload,
            full_data,
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
}
