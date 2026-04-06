# Prompt: Implement Clippy Pedantic + AI Slop Remediation DAG

Copy and paste the block below into a fresh conversation.

---

Create an aivcs episode and implement `docs/reviews/clippy-pedantic/2026-04-06/IMPLEMENTATION_DAG.toml` using subagents.

This is a 16-unit DAG for fixing 797 clippy::pedantic warnings plus 4 AI slop gap coverage units across the grokrs workspace (8 Rust crates, edition 2024, Rust 1.94).

Key context:
- The DAG has 3 layers: Layer 0 (code semantics), Layer 1 (code structure), Layer 2 (documentation), Layer 3 (validation)
- Layer 0 units are independent and can be dispatched in any order, but they share files so run sequentially
- Layer 1 depends on Layer 0. Layer 2 depends on Layers 0+1. Layer 3 depends on everything.
- The repo is at https://github.com/verivus-oss/grokrs (private, verivus-oss org)
- There are 1,663 tests currently passing — zero regressions allowed
- The deprecated `chat.rs` module produces 45 expected deprecation warnings — exclude those from the zero-warning target
- Read the full DAG TOML for acceptance criteria, critical decisions, and file lists per unit
- Review `docs/reviews/AI_SLOP_REVIEW_GUIDE.md` for context on units P13-P16 (the AI slop gap units)
- Do NOT add any AI attribution (Co-Authored-By, Claude mentions) to commits
- Run `cargo clippy --workspace --all-targets -- -W clippy::pedantic` after each unit to track progress
- Run `cargo test --workspace` after each unit to catch regressions

Verification command for the final gate (P12):
```bash
cargo clippy --workspace --all-targets -- -W clippy::pedantic 2>&1 | grep "^warning:" | grep -v "deprecated\|generated\|Compiling\|Finished" | wc -l
# Target: 0
```
