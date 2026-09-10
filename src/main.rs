//! sarg — ask sarg, tell sarg. The command line for sargineer.com.

mod agent;
mod api;
mod cli;
mod client;
mod commands;
mod config;
mod error;
mod hooks;
mod journal;
mod manifest;
mod notes;
mod output;
mod render;
mod secrets;
mod stock;

use std::process::ExitCode;
use std::time::Instant;

use clap::Parser;

/// Rust starts with SIGPIPE ignored, so `sarg api | head` would panic on
/// the write after `head` exits. Restore the default: die quietly like curl.
#[cfg(unix)]
fn reset_sigpipe() {
    // SAFETY: setting a signal disposition before any threads exist.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}
#[cfg(not(unix))]
fn reset_sigpipe() {}

fn main() -> ExitCode {
    reset_sigpipe();
    let cli = cli::Cli::parse();
    let started = Instant::now();
    let verb = cli.verb();

    let paths = config::Paths::discover();
    let result = match &paths {
        Ok(p) => commands::run(&cli, p.clone()),
        Err(e) => Err(error::SargError::usage(e.to_string())),
    };

    let code: u8 = match &result {
        Ok(c) => (*c).clamp(0, 255) as u8,
        Err(e) => {
            output::report_error(e, cli.json);
            e.exit_code()
        }
    };
    journal::record(paths.as_ref().ok(), &verb, code, started.elapsed());
    ExitCode::from(code)
}
