//! Command surface.
//!
//! Namespaces mirror `src/ipc/client.ts` so the GUI and CLI use one vocabulary.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "pane",
    version,
    about = "Drive the Pane network debugger from the terminal",
    long_about = "Drive the Pane network debugger from the terminal.\n\n\
                  Works whether or not the desktop app is open: if an instance \
                  is running, commands go to it over its local control socket; \
                  otherwise the data directory is opened directly.\n\n\
                  Set PANE_FORMAT=json once per session for machine-readable output."
)]
pub struct Cli {
    /// Machine-readable output. Also settable per session with PANE_FORMAT=json.
    #[arg(long, global = true)]
    pub json: bool,

    /// Data directory to operate on. Defaults to $PANE_DATA_DIR, then the
    /// platform location.
    #[arg(long, global = true, env = "PANE_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Proxy running? Devices paired? adb found? CA valid? Start here.
    Doctor,

    /// Emit the command tree, filter grammars and exit codes as JSON.
    Schema,

    /// Symlink this binary onto PATH as `pane`.
    Install {
        /// Directory to link into. Defaults to /usr/local/bin, falling back to
        /// ~/.local/bin when that is not writable.
        #[arg(long)]
        dir: Option<PathBuf>,
    },

    /// Run as an MCP server over stdio, exposing Pane as agent tools.
    Mcp,

    #[command(subcommand)]
    Proxy(ProxyCmd),
    #[command(subcommand)]
    Captures(CapturesCmd),
    #[command(subcommand)]
    Rules(RulesCmd),
    /// Rule collections — toggle a whole scenario at once.
    #[command(subcommand)]
    Collections(CollectionsCmd),
    #[command(subcommand)]
    Devices(DevicesCmd),
    #[command(subcommand)]
    Logcat(LogcatCmd),
    #[command(subcommand)]
    Ca(CaCmd),

    /// Stream completed captures as NDJSON. Alias for `captures tail`.
    Tail(TailArgs),
}

#[derive(Subcommand, Debug)]
pub enum ProxyCmd {
    /// Start the MITM proxy.
    Start {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8888)]
        port: u16,
    },
    /// Stop the proxy and undo device/system proxy settings.
    Stop,
    /// Is it running, where, and how many captures so far.
    Status,
    /// Run a headless instance in the foreground until Ctrl-C.
    ///
    /// Hosts its own control socket, so `pane captures tail` in another
    /// terminal behaves identically to talking to the desktop app.
    Run {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8888)]
        port: u16,
        /// Bring the instance up without starting the proxy.
        #[arg(long)]
        no_proxy: bool,
    },
}

#[derive(clap::Args, Debug)]
pub struct TailArgs {
    /// Captures filter DSL, same string the GUI search bar takes.
    #[arg(long)]
    pub filter: Option<String>,
    /// Stop after this many matching captures. Combined with --timeout this
    /// is a complete assertion: exit 7 means the count was not reached.
    #[arg(long)]
    pub count: Option<usize>,
    /// Give up after this many seconds.
    #[arg(long)]
    pub timeout: Option<u64>,
}

#[derive(Subcommand, Debug)]
pub enum CapturesCmd {
    /// Most recent captures, oldest-first (same order as the GUI).
    List {
        #[arg(long)]
        filter: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: u32,
        /// Comma-separated field names to keep.
        #[arg(long)]
        fields: Option<String>,
        /// Emit the whole CaptureDto rather than the summary projection.
        #[arg(long)]
        full: bool,
    },
    /// Print a bare count. The cheapest assertion primitive here.
    Count {
        #[arg(long)]
        filter: Option<String>,
    },
    /// One capture, with headers.
    Get { id: String },
    /// Print a request or response body.
    Body {
        id: String,
        /// Response body (the default).
        #[arg(long, conflicts_with = "req")]
        res: bool,
        /// Request body.
        #[arg(long)]
        req: bool,
        /// Truncate to this many bytes. 0 means unlimited.
        #[arg(long, default_value_t = 8192)]
        max_bytes: u64,
        /// Write to a file instead of stdout; implies no truncation.
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Emit base64 rather than decoded bytes.
        #[arg(long)]
        base64: bool,
    },
    /// Stream completed captures as NDJSON.
    Tail(TailArgs),
    /// Export one capture.
    Export {
        id: String,
        #[arg(long, value_parser = ["curl", "har"], default_value = "curl")]
        format: String,
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Delete every capture.
    Clear {
        #[arg(long)]
        r#yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum RulesCmd {
    /// List rules.
    Ls,
    /// One rule.
    Get { selector: String },
    /// Enable a rule by name substring or id.
    Enable { selector: String },
    /// Disable a rule by name substring or id.
    Disable { selector: String },
    /// Delete a rule.
    Rm {
        selector: String,
        #[arg(long)]
        r#yes: bool,
    },
    /// Create a stub rule in one line.
    Mock {
        #[arg(long)]
        host: String,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        method: Option<String>,
        #[arg(long, default_value_t = 200)]
        status: u16,
        /// Response body, inline.
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Response body, from a file.
        #[arg(long)]
        body_file: Option<PathBuf>,
        #[arg(long, default_value = "application/json")]
        mime: String,
        #[arg(long, default_value_t = 0)]
        delay_ms: u64,
        #[arg(long)]
        name: Option<String>,
        /// Create it switched off.
        #[arg(long)]
        disabled: bool,
    },
    /// Derive a rule from a real capture, reusing its matchers.
    FromCapture {
        id: String,
        #[arg(long)]
        status: Option<u16>,
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        #[arg(long)]
        body_file: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
    },
    /// Import a `pane-rules` bundle, as written by the GUI or `rules export`.
    Import {
        file: PathBuf,
        /// Report what would be created without writing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Export rules in the GUI-compatible `pane-rules` format.
    Export {
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum CollectionsCmd {
    /// List collections with their enabled state and rule count.
    Ls,
    /// Enable a collection by name substring or id.
    ///
    /// A collection groups the rules for one scenario, so this switches the
    /// whole scenario in a single call instead of toggling each rule.
    Enable { selector: String },
    /// Disable a collection by name substring or id.
    Disable { selector: String },
    /// Disable every collection except this one — the usual way to move from
    /// one scenario to the next without leaving the previous rules live.
    Only { selector: String },
}

#[derive(Subcommand, Debug)]
pub enum DevicesCmd {
    /// Paired devices.
    Ls,
    /// Devices plugged in right now that could be paired.
    Attached,
    /// Pair a device over USB. Requires a running proxy.
    Add {
        serial: String,
        #[arg(long, value_parser = ["android", "ios"])]
        platform: Option<String>,
    },
    /// Unpair a device.
    Rm {
        selector: String,
        #[arg(long)]
        r#yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum LogcatCmd {
    /// Start the adb logcat stream for a device.
    Attach {
        #[arg(long)]
        serial: String,
    },
    /// Stop the stream.
    Detach {
        #[arg(long)]
        serial: String,
    },
    /// Query persisted rows.
    Query {
        #[arg(long)]
        serial: String,
        #[arg(long)]
        filter: Option<String>,
        #[arg(long, default_value_t = 200)]
        limit: u32,
    },
    /// PID → process name for a device.
    Pids {
        #[arg(long)]
        serial: String,
    },
    /// Delete persisted rows for a device.
    Clear {
        #[arg(long)]
        serial: String,
        #[arg(long)]
        r#yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum CaCmd {
    /// Certificate details.
    Show,
    /// Export the root certificate.
    Export {
        #[arg(long, value_parser = ["pem", "der", "qr", "mobileconfig"], default_value = "pem")]
        format: String,
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
}
