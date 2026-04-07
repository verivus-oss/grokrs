# Releasing

## crates.io

This workspace is prepared for crates.io publishing, but the first publication still needs a registry token.

Current workflow:

- manual GitHub Actions workflow: `.github/workflows/publish-crates.yml`
- required secret for bootstrap publishing: `CRATES_IO_TOKEN`

After the first publication of each crate, migrate to crates.io Trusted Publishing so the repository can use short-lived OIDC credentials instead of a long-lived token.

## GitHub Release Binaries

Tagged releases use `.github/workflows/release-binaries.yml` to:

- build `grokrs` release binaries
- package per-target archives
- generate SHA-256 checksum sidecars
- create GitHub artifact attestations with `actions/attest`
- publish assets to the corresponding GitHub release

This gives release consumers signed build provenance for the release archives. With GitHub artifact attestations alone, this establishes provenance and integrity guarantees for the release artifacts. For a stricter SLSA Build Level 3 posture, move the build logic into a reusable workflow and keep verification bound to that isolated builder.

## Recommended Release Order

Publish the library crates before the CLI crate:

1. `grokrs-core`
2. `grokrs-cap`
3. `grokrs-policy`
4. `grokrs-session`
5. `grokrs-store`
6. `grokrs-tool`
7. `grokrs-api`
8. `grokrs-cli`
