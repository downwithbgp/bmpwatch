use std::fs;
use std::path::Path;

use crate::error::DoctorError;

pub fn file_size_and_format(path: &Path) -> Result<(u64, String), DoctorError> {
    let metadata = fs::metadata(path)?;
    let size = metadata.len();

    let format = match path.extension().and_then(|e| e.to_str()) {
        Some("bz2") => "BMP compressed (bz2)".to_string(),
        Some("gz") => "BMP compressed (gzip)".to_string(),
        Some("bmpr") => "BMP replay format".to_string(),
        _ => "raw BMP frames".to_string(),
    };

    Ok((size, format))
}
