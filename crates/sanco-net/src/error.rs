use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] sanco_core::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The peer could not be reached (connection refused, timeout, ...).
    #[error("{addr} is unreachable: {reason}")]
    Unreachable { addr: String, reason: String },
    /// The peer refused our token: it belongs to another workspace or uses
    /// another sync key.
    #[error("{0} rejected this PC's sync key")]
    Unauthorized(String),
    #[error("{addr} answered {status}: {message}")]
    Status {
        addr: String,
        status: u16,
        message: String,
    },
    #[error("{addr} sent a bad reply: {reason}")]
    Protocol { addr: String, reason: String },

    #[error("invalid join code: {0}")]
    BadJoinCode(&'static str),
    #[error("{0} already exists; this PC is already set up")]
    AlreadySetUp(String),
    #[error("this PC's Tailscale address is unknown; is Tailscale running?")]
    NoAddress,
    #[error("background task failed: {0}")]
    Task(String),
}

impl Error {
    /// Problems that retrying won't fix until someone acts (wrong key, other
    /// schema, bad clock, restored peer). Shown differently from "offline".
    pub fn is_blocking(&self) -> bool {
        use sanco_core::Error as C;
        matches!(
            self,
            Error::Unauthorized(_)
                | Error::Core(
                    C::SchemaMismatch { .. }
                        | C::ClockSkew { .. }
                        | C::EpochMismatch { .. }
                        | C::PeerRewound { .. }
                        | C::OwnSite
                )
        )
    }
}
