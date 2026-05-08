use std::io;
use std::time::Duration;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Eof,
    Timeout(Duration),
    Closed,
    AlreadyTaken,
    Unsupported(&'static str),
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Eof => write!(f, "EOF"),
            Self::Timeout(d) => write!(f, "timed out after {d:?}"),
            Self::Closed => write!(f, "Io has been closed"),
            Self::AlreadyTaken => {
                write!(f, "reader/writer already taken (interact already called?)")
            }
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    // Read paths classify timeouts via `is_timeout` before `?`, so we wrap
    // unconditionally here — write-side TimedOut surfaces as `Io`, not as
    // a misreported recv `Timeout`.
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
