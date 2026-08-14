.PHONY: help install dev build test lint typecheck format clean tauri-dev tauri-build docs-dev docs-build web-run cli cli-install healthcheck-install

help:
	@echo "Pane — common dev commands:"
	@echo ""
	@echo "  make install      # Install all dependencies (Rust + JS)"
	@echo "  make dev          # Run UI in browser (no Tauri)"
	@echo "  make tauri-dev    # Run desktop app in dev mode"
	@echo "  make build        # Production builds (Rust + UI)"
	@echo "  make tauri-build  # Build desktop app for current platform"
	@echo "  make test         # Run all tests (Rust + JS)"
	@echo "  make lint         # Run linters (clippy + eslint)"
	@echo "  make typecheck    # Run TypeScript typecheck"
	@echo "  make format       # Format JS/TS via prettier"
	@echo "  make docs-dev     # Astro Starlight dev server (apps/docs)"
	@echo "  make docs-build   # Build static docs into apps/docs/dist/"
	@echo "  make web-run      # Run pane-web locally on :8744 (serves docs + landing)"
	@echo "  make clean        # Remove all build artefacts"

install:
	pnpm install
	cargo fetch

dev:
	pnpm dev

tauri-dev:
	pnpm tauri:dev

build:
	cargo build --release --workspace
	pnpm build

tauri-build:
	pnpm tauri:build

test:
	cargo test --locked --workspace
	pnpm test

# --locked mirrors CI. If this fails with "the lock file needs to be updated",
# run `cargo update -p <crate>` (or plain `cargo build`) and commit Cargo.lock —
# don't drop the flag. See the alloc-stdlib pin in src-tauri/Cargo.toml for what
# an unpinned re-resolve did to the release builds.
lint:
	cargo fmt --all -- --check
	cargo clippy --locked --workspace --all-targets -- -D warnings
	pnpm lint

typecheck:
	pnpm typecheck

format:
	pnpm format

docs-dev:
	pnpm --filter @pane/docs dev

docs-build:
	pnpm --filter @pane/docs build

web-run: docs-build
	$(MAKE) -C apps/web run

# The daily liveness check runs under launchd, and macOS TCC denies a launchd
# agent any access to ~/Documents — so the script cannot execute from the repo
# checkout. Install a copy outside it and re-run this after editing the script.
healthcheck-install:
	mkdir -p $(HOME)/.local/bin
	install -m 755 scripts/pane-web-healthcheck.sh $(HOME)/.local/bin/pane-web-healthcheck.sh
	@echo "installed -> ~/.local/bin/pane-web-healthcheck.sh (launchd agent: com.pane.healthcheck)"

clean:
	cargo clean
	rm -rf node_modules dist apps/docs/dist apps/docs/node_modules

# ---- CLI ------------------------------------------------------------------
# The cargo bin is `pane-cli` (src-tauri's package already owns the `pane` bin
# name); `install` symlinks it onto PATH as `pane`.
cli:
	cargo build --release -p pane-cli

cli-install: cli
	./target/release/pane-cli install

# Bundling the CLI into the .app via tauri.conf.json `externalBin` is
# deliberately NOT wired up yet: Tauri validates that
# src-tauri/binaries/pane-cli-<target-triple> exists at build time, so adding it
# breaks `make tauri-dev` for anyone who has not cross-built the CLI first.
# Release ships it as a standalone asset instead (see .github/workflows/release.yml).
