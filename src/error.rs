//! One error type, one exit-code table.
//!
//! 0 ok · 1 error · 2 usage · 3 auth · 4 validation/blocked · 5 spooled offline

use std::fmt;

#[derive(Debug)]
pub enum SargError {
    /// Bad arguments or state the caller controls (exit 2).
    Usage(String),
    /// No token, or the server rejected it (exit 3).
    Auth { message: String, hint: Option<String> },
    /// The payload failed validation, locally or on the server (exit 4).
    Validation { message: String, hint: Option<String> },
    /// Network unreachable; anything that could be spooled has been (exit 5).
    Offline(String),
    /// Any other non-2xx answer (exit 1).
    Server {
        status: u16,
        message: String,
        hint: Option<String>,
    },
    /// Everything else (exit 1).
    Other(anyhow::Error),
}

pub type Result<T> = std::result::Result<T, SargError>;

impl SargError {
    pub fn exit_code(&self) -> u8 {
        match self {
            SargError::Usage(_) => 2,
            SargError::Auth { .. } => 3,
            SargError::Validation { .. } => 4,
            SargError::Offline(_) => 5,
            SargError::Server { .. } | SargError::Other(_) => 1,
        }
    }

    pub fn hint(&self) -> Option<&str> {
        match self {
            SargError::Auth { hint, .. }
            | SargError::Validation { hint, .. }
            | SargError::Server { hint, .. } => hint.as_deref(),
            _ => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            SargError::Usage(_) => "usage",
            SargError::Auth { .. } => "auth",
            SargError::Validation { .. } => "validation",
            SargError::Offline(_) => "offline",
            SargError::Server { .. } => "server",
            SargError::Other(_) => "error",
        }
    }

    pub fn other<E: Into<anyhow::Error>>(e: E) -> Self {
        SargError::Other(e.into())
    }

    pub fn usage<S: Into<String>>(s: S) -> Self {
        SargError::Usage(s.into())
    }
}

impl fmt::Display for SargError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SargError::Usage(m) | SargError::Offline(m) => write!(f, "{m}"),
            SargError::Auth { message, .. } | SargError::Validation { message, .. } => {
                write!(f, "{message}")
            }
            SargError::Server {
                status, message, ..
            } => write!(f, "HTTP {status}: {message}"),
            SargError::Other(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for SargError {}

impl From<anyhow::Error> for SargError {
    fn from(e: anyhow::Error) -> Self {
        SargError::Other(e)
    }
}

impl From<std::io::Error> for SargError {
    fn from(e: std::io::Error) -> Self {
        SargError::Other(e.into())
    }
}

impl From<serde_json::Error> for SargError {
    fn from(e: serde_json::Error) -> Self {
        SargError::Other(e.into())
    }
}

impl From<toml::de::Error> for SargError {
    fn from(e: toml::de::Error) -> Self {
        SargError::Other(e.into())
    }
}

impl From<toml::ser::Error> for SargError {
    fn from(e: toml::ser::Error) -> Self {
        SargError::Other(e.into())
    }
}
