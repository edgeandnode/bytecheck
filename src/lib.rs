//! `bytecheck` library root.
//!
//! The comparison engine (`artifact`, `normalize`, `compare`, `label`, `report`)
//! is network-free and deterministic; the only I/O edges are `artifact` (disk)
//! and `chain` (RPC). This split keeps the engine golden-testable and is the
//! seam along which a future `bytecheck-core` crate would be extracted.

pub mod artifact;
pub mod chain;
pub mod cli;
pub mod commands;
pub mod compare;
pub mod error;
pub mod label;
pub mod metadata;
pub mod normalize;
pub mod report;

pub use error::Error;

use cli::{Cli, Command};

/// Dispatch a parsed CLI invocation, returning the process exit code.
///
/// Async commands get a current-thread Tokio runtime; the engine itself is sync.
pub fn run(cli: Cli) -> Result<i32, Error> {
    match cli.command {
        Command::Verify(args) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| Error::Operational(format!("tokio runtime: {e}")))?;
            rt.block_on(commands::verify::run(args))
        }
    }
}
