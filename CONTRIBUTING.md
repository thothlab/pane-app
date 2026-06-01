# Contributing

## Branches & commits

- Branches: `codex/prd-NN-<slug>` for PRD-driven work, `fix/<slug>` for hotfixes, `chore/<slug>` for housekeeping.
- Commits: Conventional Commits (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`).

## Local checks before pushing

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm typecheck
pnpm test
```

CI runs the same checks on three OSes — a green local pass should mean a green CI.

## Where things live

User-facing documentation lives in `apps/docs/` (Astro Starlight, published to https://pane.thothlab.tech/docs/). The pane-web service that hosts the landing + docs + release endpoints is in `apps/web/`.
