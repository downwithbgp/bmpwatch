use std::io;

#[derive(Debug)]
pub enum DoctorError {
    Io(io::Error),
    Frame(String),
}

impl std::fmt::Display for DoctorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DoctorError::Io(e) => write!(f, "I/O error: {e}"),
            DoctorError::Frame(msg) => write!(f, "Frame error: {msg}"),
        }
    }
}

impl std::error::Error for DoctorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DoctorError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for DoctorError {
    fn from(e: io::Error) -> Self {
        DoctorError::Io(e)
    }
}
