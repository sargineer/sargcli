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

/// `sarg ask heltec | head -3` is how an agent keeps output short, and it
/// used to lose the search: SIGPIPE was restored to its default, so the
/// process died mid-render and `journal::record` below never ran. The guard
/// then could not tell that the agent had looked, and held the next flash
/// anyway — the search-then-blocked-anyway loop.
///
/// So leave SIGPIPE ignored (Rust's default) and catch the broken-pipe panic
/// the write raises instead: journal the call, then exit quietly like curl.
fn quiet_on_broken_pipe(paths: Option<config::Paths>, verb: String, started: Instant) {
    let next = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info.to_string();
        if msg.contains("Broken pipe") || msg.contains("failed printing to std") {
            journal::record(paths.as_ref(), &verb, 0, started.elapsed());
            std::process::exit(0);
        }
        next(info);
    }));
}

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    let started = Instant::now();
    let verb = cli.verb();

    let paths = config::Paths::discover();
    quiet_on_broken_pipe(paths.as_ref().ok().cloned(), verb.clone(), started);
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
