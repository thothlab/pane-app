//! `pane` — drive the Pane network debugger from a terminal.
//!
//! Deliberately *not* carrying `windows_subsystem = "windows"` the way
//! src-tauri's main does: that attribute detaches from the console, and a CLI
//! that silently prints nothing on Windows is worse than one that doesn't
//! build there.

mod cli;
mod headless;
mod install;
mod logcat_app;
mod mcp;
mod output;
mod portfile;
mod run;
mod schema;
mod session;

use clap::Parser;

use crate::output::Format;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args = cli::Cli::parse();
    let format = Format::resolve(args.json);
    init_logging();

    let code = match run::run(args, format).await {
        Ok(code) => code,
        Err(e) => output::report_error(&e, format),
    };
    std::process::exit(code);
}

/// Logs go to **stderr**, never stdout.
///
/// The GUI mirrors tracing to stdout, which is fine for a windowed app; here
/// it would corrupt every `pane … --json | jq` pipeline. Default level is
/// `warn` so normal runs are silent.
fn init_logging() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("PANE_LOG")
        .or_else(|_| EnvFilter::try_from_env("MYCHARLES_LOG"))
        .unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}
