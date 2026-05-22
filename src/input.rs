use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::str::FromStr;

use crate::error::DoctorError;
use crate::obmp_writer::MAGIC;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    RawBmp,
    Bmpd,
    Auto,
}

impl InputFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            InputFormat::RawBmp => "raw-bmp",
            InputFormat::Bmpd => "bmpd",
            InputFormat::Auto => "auto",
        }
    }
}

impl FromStr for InputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "raw-bmp" => Ok(InputFormat::RawBmp),
            "bmpd" => Ok(InputFormat::Bmpd),
            "auto" => Ok(InputFormat::Auto),
            other => Err(format!(
                "unknown format '{other}'. Expected 'raw-bmp', 'bmpd', or 'auto'"
            )),
        }
    }
}

impl std::fmt::Display for InputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Auto-detect the input format from file content.
///
/// Reads the first byte(s) of the file to determine the format:
/// - Starts with `BMPDOPENBMP1\n` → Bmpd
/// - Starts with `0x03` (BMP version 3) → RawBmp
/// - Otherwise (unknown, short, empty) → RawBmp (diagnostic fallback: the
///   parser will report appropriate errors)
pub fn detect_format(path: &Path) -> Result<InputFormat, DoctorError> {
    let mut file = File::open(path)?;
    let mut buf = [0u8; MAGIC.len()];
    let n = file.read(&mut buf)?;

    if n >= MAGIC.len() && buf == *MAGIC {
        return Ok(InputFormat::Bmpd);
    }
    if n >= 1 && buf[0] == 0x03 {
        return Ok(InputFormat::RawBmp);
    }
    // Diagnostic fallback: let the raw BMP parser report the error
    Ok(InputFormat::RawBmp)
}

pub fn file_size_and_format(path: &Path, fmt: InputFormat) -> Result<(u64, String), DoctorError> {
    assert_ne!(
        fmt,
        InputFormat::Auto,
        "format must be resolved before calling file_size_and_format"
    );
    let metadata = fs::metadata(path)?;
    let size = metadata.len();

    let format_str = match fmt {
        InputFormat::Auto => unreachable!(),
        InputFormat::RawBmp => match path.extension().and_then(|e| e.to_str()) {
            Some("bz2") => "BMP compressed (bz2)".to_string(),
            Some("gz") => "BMP compressed (gzip)".to_string(),
            Some("bmpr") => "BMP replay format".to_string(),
            _ => "raw BMP frames".to_string(),
        },
        InputFormat::Bmpd => "BMPDoctor container".to_string(),
    };

    Ok((size, format_str))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_detect_format_obmp() {
        let data = b"BMPDOPENBMP1\nrest of file...";
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(data).unwrap();
        let path = tmp.into_temp_path();
        assert_eq!(detect_format(&path).unwrap(), InputFormat::Bmpd);
    }

    #[test]
    fn test_detect_format_raw_bmp() {
        let data = b"\x03\x00\x00\x00\x06\x04rest of frame...";
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(data).unwrap();
        let path = tmp.into_temp_path();
        assert_eq!(detect_format(&path).unwrap(), InputFormat::RawBmp);
    }

    #[test]
    fn test_detect_format_unknown_fallback() {
        let data = b"NOT ANY BMP DATA HERE";
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(data).unwrap();
        let path = tmp.into_temp_path();
        assert_eq!(detect_format(&path).unwrap(), InputFormat::RawBmp);
    }

    #[test]
    fn test_detect_format_empty_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.into_temp_path();
        assert_eq!(detect_format(&path).unwrap(), InputFormat::RawBmp);
    }

    #[test]
    fn test_detect_format_short_file() {
        let data = b"\x03";
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(data).unwrap();
        let path = tmp.into_temp_path();
        assert_eq!(detect_format(&path).unwrap(), InputFormat::RawBmp);
    }
}
