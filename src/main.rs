//! `bytecheck` binary entrypoint: parse args, dispatch, map errors to exit codes.

use bytecheck::cli::Cli;
use clap::Parser;

fn main() {
    let cli = Cli::parse();
    match bytecheck::run(cli) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(err.exit_code());
        }
    }
}
