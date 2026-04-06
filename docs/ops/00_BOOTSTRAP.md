# Bootstrap Guide

## Local Setup

```bash
cd /srv/repos/internal/verivusai-labs/grokrs
cargo fmt --all
cargo test
cargo run -p grokrs-cli -- doctor
```

## AIVCS

If the local AIVCS binary is present:

```bash
/srv/repos/internal/verivusai-labs/aivcs/target/release/aivcs-cli init
/srv/repos/internal/verivusai-labs/aivcs/target/release/aivcs-cli record -m "Bootstrap grokrs scaffold"
```

## sqry

Build or refresh the semantic index after structural changes:

```bash
sqry index .
```

## Review Artifacts

The initial machine-readable artifacts live under:

`docs/reviews/bootstrap/2026-04-05/`

