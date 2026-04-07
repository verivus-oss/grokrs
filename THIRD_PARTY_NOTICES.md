# Third-Party Notices

`grokrs` is distributed under the MIT License. It also depends on third-party Rust crates that are distributed under their own licenses.

## Dependency License Policy

This repository currently accepts the following dependency licenses for the Rust dependency graph:

- Apache-2.0
- BSD-2-Clause
- BSD-3-Clause
- BSL-1.0
- CDLA-Permissive-2.0
- ISC
- MIT
- Unicode-3.0
- Unlicense
- Zlib

The allowlist is enforced by `cargo-deny` configuration in [deny.toml](/srv/repos/internal/verivusai-labs/grokrs/deny.toml).

## Current Snapshot

The current Rust dependency graph contains `291` packages in `cargo metadata` output and no crates with missing license metadata.

Observed license expressions in the current crates.io dependency graph include:

- `MIT`
- `MIT OR Apache-2.0`
- `Apache-2.0`
- `Apache-2.0 OR MIT`
- `MIT/Apache-2.0`
- `Unicode-3.0`
- `BSD-3-Clause`
- `BSL-1.0`
- `ISC`
- `Zlib`
- `Unlicense OR MIT`
- mixed expressions that combine the approved identifiers above

Examples of mixed expressions currently present in the graph:

- `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`
- `Apache-2.0 OR ISC OR MIT`
- `MIT OR Apache-2.0 OR Zlib`
- `(MIT OR Apache-2.0) AND Unicode-3.0`

## Acknowledgement

All third-party code remains under the terms chosen by its respective authors. This file is an acknowledgement and policy summary. It is not a replacement for the original license texts shipped by upstream dependencies.

## Regeneration

Re-check the dependency graph and licensing policy with:

```bash
cargo deny check
```

For a raw metadata snapshot:

```bash
cargo metadata --format-version 1
```
