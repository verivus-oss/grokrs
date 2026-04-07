# Repository Guidelines

## Purpose

`grokrs` is a safety-first Rust-only scaffold for an agentic CLI in the Grok tooling family.

The repository should stay opinionated about:
- fail-closed policy before execution
- typed trust and rooted path handling
- explicit crate boundaries instead of monolithic orchestration
- reviewable documentation and machine-readable planning artifacts

## Primary References

- [CLAUDE.md](./CLAUDE.md) for the concise repo brief
- [ARCHITECTURE.md](./ARCHITECTURE.md) for the security and crate model
- [docs/specs/00_SPEC.md](./docs/specs/00_SPEC.md) for product scope
- [docs/reviews/bootstrap/2026-04-05/IMPLEMENTATION_DAG.toml](./docs/reviews/bootstrap/2026-04-05/IMPLEMENTATION_DAG.toml) for bootstrap work decomposition

## Working Standard

Changes should favor:
- typed safety invariants over runtime convention
- small crate-local APIs over cross-cutting helpers
- explicit policy decisions over implicit shell convenience
- auditable docs and artifacts over hand-waved intent

## Expected Commands

```bash
cargo fmt --all
cargo test
cargo clippy --workspace --all-targets
cargo run -p grokrs-cli -- doctor
cargo run -p grokrs-cli -- show-config configs/grokrs.example.toml
```

## Guardrails

- Do not add non-Rust runtime dependencies for core execution paths unless there is a hard justification.
- Keep shell and network access behind policy evaluation and explicit future approval surfaces.
- Preserve the separation between capabilities, policy, session, tool, and CLI crates.
- Keep review artifacts and docs aligned when changing safety posture or crate responsibilities.
- Do not store API keys in repo files, checked-in config, or plaintext local auth files. Secrets must remain in approved secret managers or the process environment only.
