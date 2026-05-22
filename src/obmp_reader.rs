use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use crate::error::DoctorError;
use crate::obmp_writer::MAGIC;
use crate::raw_bmp::{parse_frame_from_bytes, RawBmpFrame};

#[derive(Debug)]
pub struct ObmpReader {
    reader: BufReader<File>,
    file_offset: u64,
    frame_index: u64,
    eof: bool,
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
                        "Invalid .obmp magic at offset 0: expected {:?}, got {:?}",
                        std::str::from_utf8(MAGIC).unwrap_or("<binary>"),
                        std::str::from_utf8(&magic_buf).unwrap_or("<binary>"),
                    )));
                }
            }
            Err(e) => {
                return Err(DoctorError::Frame(format!(
                    "Cannot read .obmp magic header: {e}"
                )));
            }
        }

        Ok(ObmpReader {
            reader,
            file_offset: MAGIC.len() as u64,
            frame_index: 0,
            eof: false,
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
                        "Truncated length prefix at .obmp offset {_record_start}: {e}"
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
                    "Truncated payload at .obmp offset {}: declared length {payload_len} exceeds available data: {e}",
                    self.file_offset
                ))));
            }
        }

        let frame_offset = self.file_offset;
        self.file_offset += payload_len as u64;
        let idx = self.frame_index;
        self.frame_index += 1;

        match parse_frame_from_bytes(&payload, frame_offset) {
            Ok(mut frame) => {
                frame.offset = frame_offset;
                Some(Ok(frame))
            }
            Err(e) => Some(Err(DoctorError::Frame(format!(
                ".obmp frame {idx} at offset {frame_offset}: {e}"
            )))),
        }
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
        buf.extend_from_slice(b"short"); // less than declared
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
        assert!(err.contains("Invalid .obmp magic"));
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
        // Create a valid .obmp wrapper around an invalid BMP frame
        let malformed = vec![0xFF, 0x00, 0x00, 0x00, 0x0A, 0x00, 1, 2, 3, 4]; // invalid version
        let data = fixtures::make_valid_obmp(&[malformed]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        let path = tmp.into_temp_path();

        let reader = ObmpReader::open(&path).unwrap();
        let frames: Vec<_> = reader.collect();
        assert_eq!(frames.len(), 1);
        let frame = frames[0].as_ref().unwrap();
        assert_eq!(frame.version, 0xFF); // frame parsed despite bad version
        assert_eq!(frame.msg_type_raw, 0);
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
}
