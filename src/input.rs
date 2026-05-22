use std::fs;
use std::path::Path;
use std::str::FromStr;

use crate::error::DoctorError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    RawBmp,
    OpenBmpLen,
}

impl InputFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            InputFormat::RawBmp => "raw-bmp",
            InputFormat::OpenBmpLen => "openbmp-len",
        }
    }
}

impl FromStr for InputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "raw-bmp" => Ok(InputFormat::RawBmp),
            "openbmp-len" => Ok(InputFormat::OpenBmpLen),
            other => Err(format!(
                "unknown format '{other}'. Expected 'raw-bmp' or 'openbmp-len'"
            )),
        }
    }
}

impl std::fmt::Display for InputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

pub fn file_size_and_format(path: &Path, fmt: InputFormat) -> Result<(u64, String), DoctorError> {
    let metadata = fs::metadata(path)?;
    let size = metadata.len();

    let format_str = match fmt {
        InputFormat::RawBmp => match path.extension().and_then(|e| e.to_str()) {
            Some("bz2") => "BMP compressed (bz2)".to_string(),
            Some("gz") => "BMP compressed (gzip)".to_string(),
            Some("bmpr") => "BMP replay format".to_string(),
            _ => "raw BMP frames".to_string(),
        },
        InputFormat::OpenBmpLen => "OpenBMP length-delimited".to_string(),
    };

    Ok((size, format_str))
}
