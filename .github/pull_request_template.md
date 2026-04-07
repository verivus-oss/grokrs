## Summary

Describe the change in concrete terms.

## Safety Review

- [ ] I reviewed trust, policy, path, or execution implications if this change touches them
- [ ] I updated docs or review artifacts when behavior or safety posture changed
- [ ] I did not add a new dangerous default without explicit justification

## Verification

- [ ] `cargo fmt --all`
- [ ] `cargo test`
- [ ] `cargo clippy --workspace --all-targets`
- [ ] `cargo run -p grokrs-cli -- doctor`
- [ ] `cargo run -p grokrs-cli -- show-config configs/grokrs.example.toml`

## Notes

List known risks, follow-ups, or intentionally deferred work.

