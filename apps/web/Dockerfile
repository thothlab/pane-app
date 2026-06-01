# Multi-stage build: chef caches deps so a code-only change doesn't
# rebuild the whole world. Final image is `gcr.io/distroless/cc` —
# minimal libc + the binary, no shell, ~25 MB.

FROM rust:1.85-bookworm AS chef
RUN cargo install cargo-chef --locked --version 0.1.71
WORKDIR /build

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY apps/web/Cargo.toml apps/web/Cargo.toml
COPY crates/ crates/
# src-tauri is a workspace member but pane-web doesn't depend on it,
# so we ship just enough scaffolding (Cargo.toml + stub bin + stub
# build.rs) to satisfy `cargo metadata`. The real sources arrive in
# the builder stage's `COPY . .` and overwrite these.
COPY src-tauri/Cargo.toml src-tauri/Cargo.toml
RUN mkdir -p src-tauri/src \
 && echo 'fn main() {}' > src-tauri/src/main.rs \
 && echo 'pub fn run() {}' > src-tauri/src/lib.rs \
 && echo 'fn main() {}' > src-tauri/build.rs
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p pane-web
COPY . .
RUN cargo build --release -p pane-web

FROM gcr.io/distroless/cc-debian12
COPY --from=builder /build/target/release/pane-web /usr/local/bin/pane-web
ENV PANE_BIND=0.0.0.0:8744 \
    PANE_DATA_DIR=/var/lib/pane-web \
    PANE_DOCS_DIR=/srv/pane-docs \
    PANE_DOWNLOADS_BASE=https://github.com/thothlab/pane-app/releases/latest/download \
    RUST_LOG=pane_web=info,tower_http=info
EXPOSE 8744
# Runs as root inside the container so it can create $PANE_DATA_DIR
# under the mounted volume (Docker mounts volumes root-owned by
# default). Container itself is unprivileged on the host and only
# binds 127.0.0.1, so the blast radius is small. TODO: switch back
# to nonroot once we have an init-step that chowns the volume.
ENTRYPOINT ["/usr/local/bin/pane-web"]
