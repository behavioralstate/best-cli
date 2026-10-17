mod app;
mod catalogue;
mod cli;
mod credentials;
mod http;
mod manifest;
mod output;
mod signin;

use std::process::ExitCode;

use clap::Parser;
use serde_json::json;

use crate::cli::Cli;

/// Exit codes, so a script can tell failures apart.
mod exit {
    pub const FAILED: u8 = 1;
    /// The command line was wrong (clap uses it too).
    pub const USAGE: u8 = 2;
    /// Not signed in, the sign-in was refused, or the credential was.
    pub const NOT_SIGNED_IN: u8 = 3;
    /// A high-impact command was not confirmed.
    pub const NOT_CONFIRMED: u8 = 4;
    /// The service refused the request, in its own words.
    pub const REFUSED: u8 = 5;
    /// The service could not be reached, or failed.
    pub const UNREACHABLE: u8 = 6;
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.global.json;
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(exit::FAILED);
        }
    };
    match runtime.block_on(app::run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        // `best … | head`: the reader went away, which is not a failure.
        Err(e) if is_broken_pipe(&e) => ExitCode::SUCCESS,
        Err(e) => {
            let (code, service_code, details) = classify(&e);
            if json {
                let message = service_message(&e).unwrap_or_else(|| format!("{e:#}"));
                let body = json!({ "error": { "code": service_code, "message": message, "details": details, "status": service_status(&e) }, "exit": code });
                eprintln!("{body}");
            } else {
                eprintln!("error: {e:#}");
            }
            ExitCode::from(code)
        }
    }
}

/// The exit code for an error, with the service's error code and details when it sent them.
fn classify(e: &anyhow::Error) -> (u8, Option<String>, Option<serde_json::Value>) {
    for cause in e.chain() {
        if cause.is::<output::NotSignedIn>() || cause.is::<signin::Denied>() {
            return (exit::NOT_SIGNED_IN, None, None);
        }
        if cause.is::<output::Usage>() {
            return (exit::USAGE, None, None);
        }
        if cause.is::<output::NotConfirmed>() {
            return (exit::NOT_CONFIRMED, None, None);
        }
        if let Some(h) = cause.downcast_ref::<http::Error>() {
            let code = h.code().map(str::to_owned);
            let details = h.details().cloned();
            let exit = match h {
                http::Error::Unauthorized { .. } => exit::NOT_SIGNED_IN,
                http::Error::Refused { .. } => exit::REFUSED,
                http::Error::Failed { .. } | http::Error::Unreachable(_) => exit::UNREACHABLE,
                http::Error::Unexpected(_) => exit::FAILED,
            };
            return (exit, code, details);
        }
    }
    (exit::FAILED, None, None)
}

fn service_message(e: &anyhow::Error) -> Option<String> {
    e.chain()
        .find_map(|c| c.downcast_ref::<http::Error>())
        .map(http::Error::message)
}

fn service_status(e: &anyhow::Error) -> Option<u16> {
    e.chain()
        .find_map(|c| c.downcast_ref::<http::Error>())
        .and_then(http::Error::status)
}

fn is_broken_pipe(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
            || c.downcast_ref::<serde_json::Error>()
                .is_some_and(|j| j.io_error_kind() == Some(std::io::ErrorKind::BrokenPipe))
    })
}
