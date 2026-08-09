//! Embed the built SPA (`dist/`) into the binary.
//!
//! `include_bytes!` off a generated file rather than `rust-embed` or
//! `include_dir`: neither is in `Cargo.lock`, CI builds `--locked`, and the
//! feature that would justify one — reloading assets from disk in development —
//! is useless here because `pnpm dev` serves the SPA from Vite instead.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dist");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("assets.rs");

    // Re-run when the directory itself changes, so a brand new file is noticed.
    // Per-file directives below cover *edits*; this covers additions and
    // deletions.
    println!("cargo:rerun-if-changed={}", dist.display());

    let mut files = Vec::new();
    collect(&dist, &mut files);
    files.sort();

    let mut src = String::new();
    if files.is_empty() {
        // Deliberately not a hard error. CI's Rust jobs stub dist/index.html
        // for the Tauri macro and never run `pnpm build`, and a developer who
        // has not built the frontend should still get a compiling workspace.
        // The server reports this at runtime instead.
        println!(
            "cargo:warning=pane-serve: no frontend bundle at {} — \
             `pane serve` will report that the UI was not built. Run `pnpm build`.",
            dist.display()
        );
        src.push_str("pub const DIST_PRESENT: bool = false;\n");
    } else {
        src.push_str("pub const DIST_PRESENT: bool = true;\n");
    }

    src.push_str("pub static ASSETS: &[(&str, &str, &[u8])] = &[\n");
    for path in &files {
        // Vite rewrites hashed chunk filenames without touching the directory
        // mtime, so every file needs its own directive. Missing this ships a
        // silently stale bundle, which is the nastiest failure mode here.
        println!("cargo:rerun-if-changed={}", path.display());

        let rel = path.strip_prefix(&dist).expect("under dist");
        let url = format!("/{}", rel.to_string_lossy().replace('\\', "/"));
        let _ = writeln!(
            src,
            "    ({:?}, {:?}, include_bytes!({:?})),",
            url,
            content_type(path),
            path.to_string_lossy(),
        );
    }
    src.push_str("];\n");

    std::fs::write(&out, src).expect("writing assets.rs");
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

/// Content type by extension.
///
/// Hand-rolled rather than pulling `mime_guess` in as a build dependency: a
/// Vite bundle is a closed set of extensions, and getting `text/javascript`
/// and `charset=utf-8` right for those is the whole job. An unknown extension
/// falls back to `application/octet-stream`, which a browser will refuse to
/// execute — the correct outcome for something we did not expect to ship.
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") | Some("map") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}
