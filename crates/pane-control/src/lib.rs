//! Local control endpoint for a running Pane instance.
//!
//! Lets an external process — the `pane` CLI, the MCP server, a CI script —
//! drive the instance that already owns the data directory, instead of
//! fighting it for the SQLite file and port 8888.
//!
//! # Transport
//!
//! A Unix domain socket at `<data_dir>/control.sock`, mode 0600, with a
//! `control.json` next to it describing where to find it.
//!
//! No token. On a Unix socket the permission check is kernel-enforced against
//! the connecting uid, and a token would add no boundary a same-uid process
//! could not already cross by reading the token file. (The one-shot token in
//! `pane-setup-server` is not a counter-example: that server binds a LAN
//! address, so it has a remote attacker to authenticate. This one does not.)
//!
//! Loopback TCP was the alternative and is worse here: any local process can
//! connect to it regardless of uid, and a random port needs a discovery file
//! anyway — so it would carry all of this machinery *plus* a secret to manage.

pub mod client;
pub mod discovery;
pub mod dispatch;
pub mod protocol;
pub mod server;

pub use client::{Client, ConnectError};
pub use discovery::{Discovery, InstanceKind};
pub use protocol::{EventFrame, Request, Response, SubscribeArgs, PROTOCOL_VERSION};
pub use server::ControlServer;
