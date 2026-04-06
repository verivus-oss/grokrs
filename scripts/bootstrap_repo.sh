#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

cargo fmt --all
cargo test

if command -v cargo >/dev/null 2>&1; then
  cargo clippy --workspace --all-targets
fi

if [[ -x /srv/repos/internal/verivusai-labs/aivcs/target/release/aivcs-cli ]] && [[ ! -d .aivcs ]]; then
  /srv/repos/internal/verivusai-labs/aivcs/target/release/aivcs-cli init
fi

echo "Bootstrap complete for ${repo_root}"

