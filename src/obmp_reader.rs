use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use bytes::Bytes;

use crate::error::DoctorError;
use crate::obmp_writer::MAGIC;
use crate::raw_bmp::{parse_frame_from_bytes, RawBmpFrame};

/// Payload classification for `.bmpd` container records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadKind {
    RawBmp,
    OpenBmpWrapped,
    Unrecognized,
}

/// Container-level statistics for `.bmpd` files.
#[derive(Debug, Clone, Default)]
pub struct ContainerStats {
    pub container_records: u64,
    pub raw_bmp_payloads: u64,
    pub openbmp_wrapped_payloads: u64,
    pub unrecognized_payloads: u64,
    pub openbmp_unwrap_errors: u64,
    pub inner_bmp_parse_errors: u64,
    pub openbmp_metadata: Option<OpenBmpMetadata>,
}

impl ContainerStats {
    pub fn has_data(&self) -> bool {
        self.container_records > 0
    }

    fn push_metadata(&mut self, meta: OpenBmpMetadata) {
        if self.openbmp_metadata.is_none() {
            self.openbmp_metadata = Some(meta);
        }
    }
}

/// Normalized OpenBMP metadata extracted from the first successfully
/// unwrapped `OBMP` payload in a `.bmpd` container.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct OpenBmpMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router_ip: Option<String>,
}

impl OpenBmpMetadata {
    pub fn any(&self) -> bool {
        self.collector.is_some() || self.router.is_some() || self.router_ip.is_some()
    }
}

/// Result of unwrapping an OpenBMP payload.
struct UnwrapResult {
    inner_bytes: Vec<u8>,
    metadata: OpenBmpMetadata,
}

/// Unwrap an OpenBMP header from a payload and return the inner raw BMP bytes
/// plus normalized metadata from the OpenBMP header.
///
/// RouteViews Kafka `*.bmp_raw` messages start with an OpenBMP wrapper
/// (`OBMP` magic + header) before the inner RFC 7854 BMP frame. This
/// function uses bgpkit-parser to strip the OpenBMP header, leaving the
/// inner BMP bytes for our existing frame parser.
fn try_unwrap_openbmp(payload: &[u8]) -> Result<UnwrapResult, String> {
    if payload.len() < 4 || &payload[..4] != b"OBMP" {
        return Err("not an OpenBMP message".to_string());
    }

    let mut data = Bytes::copy_from_slice(payload);
    match bgpkit_parser::parser::bmp::openbmp::parse_openbmp_header(&mut data) {
        Ok(header) => {
            let inner_bytes = data.to_vec();
            if inner_bytes.is_empty() {
                return Err("OpenBMP header parsed but no inner BMP frame".to_string());
            }
            let metadata = OpenBmpMetadata {
                collector: if header.admin_id.is_empty() {
                    None
                } else {
                    Some(header.admin_id)
                },
                router: header.router_group.filter(|g| !g.is_empty()),
                router_ip: Some(header.router_ip.to_string()),
            };
            Ok(UnwrapResult {
                inner_bytes,
                metadata,
            })
        }
        Err(e) => Err(format!("Malformed OpenBMP wrapper: {e}")),
    }
}

/// Result of parsing a `.bmpd` record payload.
struct RecordResult {
    frame: Result<RawBmpFrame, DoctorError>,
    kind: PayloadKind,
    metadata: Option<OpenBmpMetadata>,
}

/// Parse a record payload that may be raw BMP or OpenBMP-wrapped.
///
/// Returns the parsed frame (or error), the payload kind for container stats,
/// and OpenBMP metadata if the payload was OpenBMP-wrapped and unwrapped
/// successfully.
fn parse_record_payload(payload: &[u8], frame_offset: u64, frame_index: u64) -> RecordResult {
    if payload.is_empty() {
        return RecordResult {
            frame: Err(DoctorError::Frame(format!(
                ".bmpd frame {frame_index} at offset {frame_offset}: empty payload"
            ))),
            kind: PayloadKind::Unrecognized,
            metadata: None,
        };
    }

    let kind = if payload[0] == 0x03 {
        PayloadKind::RawBmp
    } else if payload.len() >= 4 && &payload[..4] == b"OBMP" {
        PayloadKind::OpenBmpWrapped
    } else {
        PayloadKind::Unrecognized
    };

    let (frame, metadata) = match kind {
        PayloadKind::RawBmp => (
            parse_frame_from_bytes(payload, frame_offset)
                .map(|mut frame| {
                    frame.offset = frame_offset;
                    frame
                })
                .map_err(|e| {
                    DoctorError::Frame(format!(
                        ".bmpd frame {frame_index} at offset {frame_offset}: {e}"
                    ))
                }),
            None,
        ),
        PayloadKind::OpenBmpWrapped => match try_unwrap_openbmp(payload) {
            Ok(result) => (
                parse_frame_from_bytes(&result.inner_bytes, frame_offset)
                    .map(|mut frame| {
                        frame.offset = frame_offset;
                        frame
                    })
                    .map_err(|e| {
                        DoctorError::Frame(format!(
                            ".bmpd frame {frame_index} at offset {frame_offset}: {e}"
                        ))
                    }),
                Some(result.metadata),
            ),
            Err(msg) => (
                Err(DoctorError::Frame(format!(
                    ".bmpd frame {frame_index} at offset {frame_offset}: {msg}"
                ))),
                None,
            ),
        },
        PayloadKind::Unrecognized => (
            Err(DoctorError::Frame(format!(
                ".bmpd frame {frame_index} at offset {frame_offset}: unrecognized payload (first byte 0x{:02x})",
                payload[0]
            ))),
            None,
        ),
    };

    RecordResult {
        frame,
        kind,
        metadata,
    }
}

#[derive(Debug)]
pub struct ObmpReader {
    reader: BufReader<File>,
    file_offset: u64,
    frame_index: u64,
    eof: bool,
    pub stats: ContainerStats,
}

impl ObmpReader {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, DoctorError> {
        let file = File::open(&path)?;
        let mut reader = BufReader::new(file);

        let mut magic_buf = [0u8; MAGIC.len()];
        match reader.read_exact(&mut magic_buf) {
            Ok(()) => {
                if magic_buf != *MAGIC {
                    return Err(DoctorError::Frame(format!(
                        "Invalid .bmpd magic at offset 0: expected {:?}, got {:?}",
                        std::str::from_utf8(MAGIC).unwrap_or("<binary>"),
                        std::str::from_utf8(&magic_buf).unwrap_or("<binary>"),
                    )));
                }
            }
            Err(e) => {
                return Err(DoctorError::Frame(format!(
                    "Cannot read .bmpd magic header: {e}"
                )));
            }
        }

        Ok(ObmpReader {
            reader,
            file_offset: MAGIC.len() as u64,
            frame_index: 0,
            eof: false,
            stats: ContainerStats::default(),
        })
    }
}

impl Iterator for ObmpReader {
    type Item = Result<RawBmpFrame, DoctorError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.eof {
            return None;
        }

        let _record_start = self.file_offset;

        // Read u32 BE length prefix
        let mut len_buf = [0u8; 4];
        match self.reader.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                self.eof = true;
                if self.frame_index == 0 {
                    return Some(Err(DoctorError::Frame(format!(
                        "Truncated length prefix at .bmpd offset {_record_start}: {e}"
                    ))));
                }
                return None;
            }
            Err(e) => return Some(Err(DoctorError::Io(e))),
        }

        self.file_offset += 4;
        let payload_len = u32::from_be_bytes(len_buf) as usize;

        let mut payload = vec![0u8; payload_len];
        match self.reader.read_exact(&mut payload) {
            Ok(()) => {}
            Err(e) => {
                self.eof = true;
                return Some(Err(DoctorError::Frame(format!(
                    "Truncated payload at .bmpd offset {}: declared length {payload_len} exceeds available data: {e}",
                    self.file_offset
                ))));
            }
        }

        let frame_offset = self.file_offset;
        self.file_offset += payload_len as u64;
        let idx = self.frame_index;
        self.frame_index += 1;

        let record = parse_record_payload(&payload, frame_offset, idx);

        // Capture metadata from the first successful OpenBMP unwrap
        if self.stats.container_records == 0 {
            if let Some(ref meta) = record.metadata {
                if meta.any() {
                    self.stats.push_metadata(meta.clone());
                }
            }
        }

        // Update container stats
        self.stats.container_records += 1;
        match record.kind {
            PayloadKind::RawBmp => {
                self.stats.raw_bmp_payloads += 1;
                if record.frame.is_err() {
                    self.stats.inner_bmp_parse_errors += 1;
                }
            }
            PayloadKind::OpenBmpWrapped => {
                self.stats.openbmp_wrapped_payloads += 1;
                if let Err(e) = &record.frame {
                    let msg = e.to_string();
                    if msg.contains("Malformed OpenBMP wrapper") {
                        self.stats.openbmp_unwrap_errors += 1;
                    } else {
                        self.stats.inner_bmp_parse_errors += 1;
                    }
                }
            }
            PayloadKind::Unrecognized => {
                self.stats.unrecognized_payloads += 1;
            }
        }

        Some(record.frame)
    }
}

#[cfg(test)]
pub mod fixtures {
    use crate::raw_bmp::fixtures as bmp_fixtures;

    pub fn make_valid_obmp(frames: &[Vec<u8>]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(crate::obmp_writer::MAGIC);
        for payload in frames {
            let len = payload.len() as u32;
            buf.extend_from_slice(&len.to_be_bytes());
            buf.extend_from_slice(payload);
        }
        buf
    }

    pub fn make_obmp_with_one_peer_up(peer_asn: u32, ip: [u8; 4]) -> Vec<u8> {
        let frame = bmp_fixtures::make_peer_up_frame(peer_asn, ip, 100, 0);
        make_valid_obmp(&[frame])
    }

    pub fn make_obmp_with_magic_only() -> Vec<u8> {
        Vec::from(crate::obmp_writer::MAGIC.as_slice())
    }

    pub fn make_obmp_invalid_magic() -> Vec<u8> {
        b"INVALIDHEADERX".to_vec()
    }

    pub fn make_obmp_truncated_length() -> Vec<u8> {
        let mut buf = Vec::from(crate::obmp_writer::MAGIC.as_slice());
        buf.extend_from_slice(&[0x00, 0x00]); // only 2 of 4 length bytes
        buf
    }

    pub fn make_obmp_length_exceeds_file(declared: u32) -> Vec<u8> {
        let mut buf = Vec::from(crate::obmp_writer::MAGIC.as_slice());
        buf.extend_from_slice(&declared.to_be_bytes());
        buf.extend_from_slice(b"short");
        buf
    }

    /// Build a minimal valid OpenBMP header + inner BMP frame.
    /// Uses the header layout from bgpkit-parser's test fixture.
    /// `inner_bmp` is a complete raw BMP frame (common header + body).
    pub fn make_openbmp_wrapped(inner_bmp: &[u8]) -> Vec<u8> {
        // Known-valid OpenBMP header fields from bgpkit-parser test fixture.
        // header_start = magic(4) + version(2) + header_len(2)
        let header_start = hex::decode("4f424d5001070064").expect("hardcoded hex should decode");
        // header_tail = flags through row_count (excluding header_start + msg_len)
        let header_tail = hex::decode(
            "800c6184b9c2000c602cbf4f072f3ae149d23486024bc3dadfc4000a69732d63632d626d7031c677060bdd020a9e92be000200de2e3180df3369000000000000000000000000000c726f7574652d76696577733500000001",
        )
        .expect("hardcoded hex should decode");

        let msg_len = inner_bmp.len() as u32;

        let mut buf = header_start;
        buf.extend_from_slice(&msg_len.to_be_bytes());
        buf.extend_from_slice(&header_tail);
        buf.extend_from_slice(inner_bmp);

        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_empty_obmp_magic_only() {
        let data = fixtures::make_obmp_with_magic_only();
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let reader = ObmpReader::open(&path).unwrap();
        let frames: Vec<_> = reader.collect();
        // Magic-only file: 0 complete frames but the truncated length prefix
        // after magic produces one error.
        assert_eq!(frames.len(), 1);
        assert!(frames[0].is_err());
    }

    #[test]
    fn test_one_valid_bmp_frame() {
        let frame = crate::raw_bmp::fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let data = fixtures::make_valid_obmp(&[frame]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let reader = ObmpReader::open(&path).unwrap();
        let frames: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(frames.len(), 1);
        let f = &frames[0];
        assert_eq!(f.msg_type_raw, 3); // PeerUp
        assert!(f.per_peer_header.is_some());
        assert_eq!(f.per_peer_header.as_ref().unwrap().peer_asn, 65000);
    }

    #[test]
    fn test_multiple_valid_bmp_frames() {
        let pu = crate::raw_bmp::fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let rm =
            crate::raw_bmp::fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 200, 0);
        let data = fixtures::make_valid_obmp(&[pu, rm]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let reader = ObmpReader::open(&path).unwrap();
        let frames: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].msg_type_raw, 3);
        assert_eq!(frames[1].msg_type_raw, 0);
    }

    #[test]
    fn test_invalid_magic() {
        let data = fixtures::make_obmp_invalid_magic();
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let result = ObmpReader::open(&path);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid .bmpd magic"));
    }

    #[test]
    fn test_truncated_length_prefix() {
        let data = fixtures::make_obmp_truncated_length();
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let reader = ObmpReader::open(&path).unwrap();
        let frames: Vec<_> = reader.collect();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].is_err());
        let err = frames[0].as_ref().unwrap_err().to_string();
        assert!(err.contains("Truncated length prefix"));
    }

    #[test]
    fn test_declared_length_exceeds_file() {
        let data = fixtures::make_obmp_length_exceeds_file(100);
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let reader = ObmpReader::open(&path).unwrap();
        let frames: Vec<_> = reader.collect();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].is_err());
        let err = frames[0].as_ref().unwrap_err().to_string();
        assert!(err.contains("Truncated payload"));
    }

    #[test]
    fn test_zero_length_payload() {
        let data = fixtures::make_valid_obmp(&[vec![]]);
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let reader = ObmpReader::open(&path).unwrap();
        let frames: Vec<_> = reader.collect();
        assert_eq!(frames.len(), 1);
        // Zero-length payload is not a valid BMP frame — parse_frame_from_bytes reports error
        assert!(frames[0].is_err());
    }

    #[test]
    fn test_valid_container_malformed_bmp() {
        // Create a valid .bmpd wrapper around an invalid BMP frame (wrong version)
        let malformed = vec![0xFF, 0x00, 0x00, 0x00, 0x0A, 0x00, 1, 2, 3, 4];
        let data = fixtures::make_valid_obmp(&[malformed]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let reader = ObmpReader::open(&path).unwrap();
        let frames: Vec<_> = reader.collect();
        assert_eq!(frames.len(), 1);
        // First byte 0xFF is not 0x03 (BMP) or OBMP, so it's rejected as unrecognized
        assert!(frames[0].is_err());
    }

    #[test]
    fn test_obmp_with_initiation_frame() {
        let init = crate::raw_bmp::fixtures::make_initiation_frame("test-router");
        let data = fixtures::make_valid_obmp(&[init]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let reader = ObmpReader::open(&path).unwrap();
        let frames: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].msg_type_raw, 4); // Initiation
        assert!(frames[0].per_peer_header.is_none());
    }

    #[test]
    fn test_openbmp_wrapped_bmp_frame() {
        let inner = crate::raw_bmp::fixtures::make_peer_up_frame(65001, [10, 0, 0, 2], 500, 0);
        let wrapped = fixtures::make_openbmp_wrapped(&inner);
        let data = fixtures::make_valid_obmp(&[wrapped]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let reader = ObmpReader::open(&path).unwrap();
        let frames: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(frames.len(), 1);
        let f = &frames[0];
        assert_eq!(f.msg_type_raw, 3); // PeerUp
        assert!(f.per_peer_header.is_some());
        assert_eq!(f.per_peer_header.as_ref().unwrap().peer_asn, 65001);
        assert_eq!(f.per_peer_header.as_ref().unwrap().peer_ip(), "10.0.0.2");
    }

    #[test]
    fn test_openbmp_wrapped_then_raw_bmp_mixed() {
        let inner = crate::raw_bmp::fixtures::make_peer_up_frame(65001, [10, 0, 0, 2], 500, 0);
        let wrapped = fixtures::make_openbmp_wrapped(&inner);
        let raw =
            crate::raw_bmp::fixtures::make_route_monitoring_frame(65001, [10, 0, 0, 2], 600, 0);
        let data = fixtures::make_valid_obmp(&[wrapped, raw]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let reader = ObmpReader::open(&path).unwrap();
        let frames: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(frames.len(), 2);
        // First: OpenBMP-wrapped PeerUp
        assert_eq!(frames[0].msg_type_raw, 3);
        // Second: raw BMP RouteMonitoring
        assert_eq!(frames[1].msg_type_raw, 0);
    }

    #[test]
    fn test_openbmp_inner_bmp_malformed() {
        // Inner BMP has wrong version but valid wrapper
        let malformed_inner = vec![0xFF, 0x00, 0x00, 0x00, 0x0A, 0x00, 0u8, 0, 0, 0];
        let wrapped = fixtures::make_openbmp_wrapped(&malformed_inner);
        let data = fixtures::make_valid_obmp(&[wrapped]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let reader = ObmpReader::open(&path).unwrap();
        let frames: Vec<_> = reader.collect();
        assert_eq!(frames.len(), 1);
        let frame = frames[0].as_ref().unwrap();
        // Frame is parsed despite bad inner BMP version
        assert_eq!(frame.version, 0xFF);
    }

    #[test]
    fn test_committed_fixture_two_openbmp_records() {
        // Read-only regression test against committed fixture.
        // The fixture was generated by the fixture-generation helper and
        // committed; tests must not mutate it.
        let reader = ObmpReader::open("tests/fixtures/openbmp-two-records.bmpd").unwrap();
        let frames: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].msg_type_raw, 3); // PeerUp
        assert_eq!(frames[0].per_peer_header.as_ref().unwrap().peer_asn, 65000);
        assert_eq!(frames[1].msg_type_raw, 0); // RouteMonitoring
    }

    #[test]
    fn test_container_stats_committed_fixture() {
        // Read-only stats check against committed fixture.
        let mut reader = ObmpReader::open("tests/fixtures/openbmp-two-records.bmpd").unwrap();
        for _ in reader.by_ref() {}
        let stats = &reader.stats;
        assert_eq!(stats.container_records, 2);
        assert_eq!(stats.openbmp_wrapped_payloads, 2);
        assert_eq!(stats.raw_bmp_payloads, 0);
        assert_eq!(stats.unrecognized_payloads, 0);
    }

    #[test]
    fn test_container_stats_raw_bmp_payloads() {
        let raw1 = crate::raw_bmp::fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let raw2 =
            crate::raw_bmp::fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 200, 0);
        let data = fixtures::make_valid_obmp(&[raw1, raw2]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut reader = ObmpReader::open(&path).unwrap();
        for _ in reader.by_ref() {}
        let stats = &reader.stats;
        assert_eq!(stats.container_records, 2);
        assert_eq!(stats.raw_bmp_payloads, 2);
        assert_eq!(stats.openbmp_wrapped_payloads, 0);
        assert_eq!(stats.unrecognized_payloads, 0);
    }

    #[test]
    fn test_container_stats_unrecognized_payload() {
        let bad = vec![0xAB, 0xCD, 0xEF, 0x00, 0x01, 0x02]; // not 0x03, not OBMP
        let data = fixtures::make_valid_obmp(&[bad]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut reader = ObmpReader::open(&path).unwrap();
        // Drain iterator, keeping reader alive for stats access
        for _ in reader.by_ref() {}
        let stats = &reader.stats;
        assert_eq!(stats.container_records, 1);
        assert_eq!(stats.unrecognized_payloads, 1);
        assert_eq!(stats.raw_bmp_payloads, 0);
        assert_eq!(stats.openbmp_wrapped_payloads, 0);
    }

    #[test]
    fn test_container_stats_openbmp_unwrap_error() {
        // Valid OBMP magic but bad header content (wrong version)
        let bad_openbmp = {
            let mut v = b"OBMP\xFF\xFF".to_vec(); // bad version
            v.extend_from_slice(&[0u8; 50]); // garbage rest
            v
        };
        let data = fixtures::make_valid_obmp(&[bad_openbmp]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut reader = ObmpReader::open(&path).unwrap();
        for r in reader.by_ref() {
            assert!(r.is_err());
        }
        let stats = &reader.stats;
        assert_eq!(stats.container_records, 1);
        assert_eq!(stats.openbmp_wrapped_payloads, 1);
        assert_eq!(stats.openbmp_unwrap_errors, 1);
    }

    #[test]
    fn test_openbmp_metadata_populated() {
        let inner = crate::raw_bmp::fixtures::make_peer_up_frame(65001, [10, 0, 0, 2], 500, 0);
        let wrapped = fixtures::make_openbmp_wrapped(&inner);
        let data = fixtures::make_valid_obmp(&[wrapped]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut reader = ObmpReader::open(&path).unwrap();
        for _ in reader.by_ref() {}
        let stats = &reader.stats;
        assert!(stats.openbmp_metadata.is_some());
        let meta = stats.openbmp_metadata.as_ref().unwrap();
        assert_eq!(meta.collector.as_deref(), Some("is-cc-bmp1"));
        assert_eq!(meta.router.as_deref(), Some("route-views5"));
        assert!(meta.router_ip.is_some());
    }

    #[test]
    fn test_openbmp_metadata_absent_for_raw_bmp() {
        let frame = crate::raw_bmp::fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let data = fixtures::make_valid_obmp(&[frame]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut reader = ObmpReader::open(&path).unwrap();
        for _ in reader.by_ref() {}
        assert!(reader.stats.openbmp_metadata.is_none());
    }

    #[test]
    fn test_committed_fixture_has_metadata() {
        let inner1 = crate::raw_bmp::fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 1000, 0);
        let inner2 =
            crate::raw_bmp::fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 2000, 0);
        let wrapped1 = fixtures::make_openbmp_wrapped(&inner1);
        let wrapped2 = fixtures::make_openbmp_wrapped(&inner2);
        let data = fixtures::make_valid_obmp(&[wrapped1, wrapped2]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut reader = ObmpReader::open(&path).unwrap();
        for _ in reader.by_ref() {}
        let stats = &reader.stats;
        assert!(stats.openbmp_metadata.is_some());
        let meta = stats.openbmp_metadata.as_ref().unwrap();
        assert!(meta.collector.is_some());
        assert!(meta.router.is_some());
        assert!(meta.router_ip.is_some());
    }

    #[test]
    fn test_container_stats_mixed_payloads() {
        // One raw BMP record + one OpenBMP-wrapped record
        let raw = crate::raw_bmp::fixtures::make_peer_up_frame(65000, [10, 0, 0, 1], 100, 0);
        let inner =
            crate::raw_bmp::fixtures::make_route_monitoring_frame(65000, [10, 0, 0, 1], 200, 0);
        let wrapped = fixtures::make_openbmp_wrapped(&inner);
        let data = fixtures::make_valid_obmp(&[raw, wrapped]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut reader = ObmpReader::open(&path).unwrap();
        for _ in reader.by_ref() {}
        let stats = &reader.stats;
        assert_eq!(stats.container_records, 2);
        assert_eq!(stats.raw_bmp_payloads, 1);
        assert_eq!(stats.openbmp_wrapped_payloads, 1);
        assert_eq!(stats.unrecognized_payloads, 0);
    }

    #[test]
    fn test_container_stats_inner_bmp_parse_error() {
        // Valid OpenBMP wrapper around an inner BMP with msg_len < 6.
        // Unwrap succeeds but parse_frame_from_bytes rejects the inner frame.
        let malformed_inner = vec![0x03, 0x00, 0x00, 0x00, 0x01, 0x00]; // msg_len=1 < 6
        let wrapped = fixtures::make_openbmp_wrapped(&malformed_inner);
        let data = fixtures::make_valid_obmp(&[wrapped]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let mut reader = ObmpReader::open(&path).unwrap();
        for _ in reader.by_ref() {}
        let stats = &reader.stats;
        assert_eq!(stats.container_records, 1);
        assert_eq!(stats.openbmp_wrapped_payloads, 1);
        // Unwrap succeeded (valid header), inner BMP parse failed
        assert_eq!(stats.openbmp_unwrap_errors, 0);
        assert!(stats.inner_bmp_parse_errors > 0);
    }
}
