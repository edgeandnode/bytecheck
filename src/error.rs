//! Crate error type and its mapping to the CI exit-code contract (§10).

use thiserror::Error;

/// Errors surfaced to the CLI edge. The variant determines the process exit
/// code; see [`Error::exit_code`].
#[derive(Debug, Error)]
pub enum Error {
    /// Bad arguments, unreadable/unparseable artifact, bad address book → exit 2.
    #[error("usage: {0}")]
    Usage(String),

    /// Artifact present but malformed or missing required fields → exit 2.
    #[error("artifact: {0}")]
    Artifact(String),

    /// RPC unreachable, address has no code, resolution failed → exit 3.
    #[error("chain: {0}")]
    Chain(String),

    /// Anything else operational (runtime setup, serialization) → exit 3.
    #[error("operational: {0}")]
    Operational(String),
}

impl Error {
    /// Map an error to its CI exit code per the contract in `ARCHITECTURE.md` §10.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Usage(_) | Error::Artifact(_) => 2,
            Error::Chain(_) | Error::Operational(_) => 3,
        }
    }
}
