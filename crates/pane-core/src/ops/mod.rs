//! The operation surface.
//!
//! One module per namespace, mirroring `src/ipc/client.ts` so the GUI, the
//! CLI and the control protocol all use the same vocabulary. Each is an
//! `impl Core` block; the bodies came straight out of the old
//! `#[tauri::command]` functions, which are now adapters over these.

mod ca;
mod captures;
mod devices;
mod filters;
mod host;
mod logcat;
mod proxy;
mod replay;
mod rules;

pub use captures::{read_text_file, write_text_file};
pub use logcat::LogcatStartOutcome;
