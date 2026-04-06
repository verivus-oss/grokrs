# Example Policy Checks

```bash
cargo run -p grokrs-cli -- eval read README.md
cargo run -p grokrs-cli -- eval write docs/specs/00_SPEC.md
cargo run -p grokrs-cli -- eval network api.x.ai
cargo run -p grokrs-cli -- eval spawn cargo
```

Expected default posture:
- reads: allowed
- workspace writes: allowed
- network: denied
- process spawn: denied

