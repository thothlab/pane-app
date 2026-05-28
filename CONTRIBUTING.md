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

The PRD package under `docs/tasks/prd_01_mvp-network-debugger/` is the source of truth for scope and acceptance criteria. If a change would alter behaviour beyond what the PRD describes, update the PRD in the same PR.
