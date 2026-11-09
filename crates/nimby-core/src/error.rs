use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("http {status}: {url}")]
    HttpStatus { url: String, status: u16 },
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Msg(String),
    #[error("unsafe or invalid path: {0}")]
    UnsafePath(String),
    #[error("hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("not a Swifty mod manifest: {0}")]
    Manifest(String),
    #[error("missing file: {0}")]
    MissingFile(PathBuf),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn msg(s: impl Into<String>) -> Self {
        Error::Msg(s.into())
    }

    /// The user asked to stop.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Error::Cancelled)
    }

    /// Another attempt could plausibly succeed: network hiccups, timeouts, a
    /// busy disk, a truncated body or a body that failed its checksum.
    ///
    /// A 4xx (except 408/425/429) will keep failing, so it is reported straight
    /// away; the caller then decides whether to skip, retry or abort.
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Io(err) => matches!(
                err.kind(),
                std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::Interrupted
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::BrokenPipe
            ),
            // Transport level: the request never produced a usable response.
            Error::Http(_) => true,
            Error::HttpStatus { status, .. } => {
                matches!(*status, 408 | 425 | 429) || (500..600).contains(status)
            }
            Error::HashMismatch { .. } => true,
            _ => false,
        }
    }

    /// Short label for a UI: which kind of problem this is.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Error::Io(_) => "disk",
            Error::Http(_) => "network",
            Error::HttpStatus { .. } => "network",
            Error::HashMismatch { .. } => "checksum",
            Error::Manifest(_) => "manifest",
            Error::MissingFile(_) => "missing-source",
            Error::UnsafePath(_) => "path",
            Error::Json(_) => "manifest",
            Error::Cancelled => "cancelled",
            Error::Msg(_) => "unknown",
        }
    }
}
