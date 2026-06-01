.PHONY: help install dev build test lint typecheck format clean tauri-dev tauri-build docs-dev docs-build web-run

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
	cargo test --workspace
	pnpm test

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
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

clean:
	cargo clean
	rm -rf node_modules dist apps/docs/dist apps/docs/node_modules
