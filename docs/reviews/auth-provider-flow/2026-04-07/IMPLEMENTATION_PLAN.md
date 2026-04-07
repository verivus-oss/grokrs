# Auth Provider Flow Implementation Plan

## Goal

Add a first-class auth provider flow to `grokrs` so xAI credentials can be resolved securely at runtime without storing secrets in repo files, checked-in config, or plaintext local auth files.

## Scope

- inference API auth for normal `grokrs` command flows
- explicit provider metadata in config
- runtime provider chain with Azure Key Vault support
- operator-facing diagnostics for auth source and health
- docs that make the secure path the default path

## Non-Goals

- storing xAI keys in repo config or local plaintext auth caches
- replacing Azure Key Vault with a repo-local wrapper as the long-term solution
- full Management API auth integration in the first pass
- OS keyring provider support in the first pass

## Proposed Execution Order

1. Define typed config for auth providers in `grokrs-core`.
2. Build a runtime resolver abstraction in `grokrs-api`.
3. Implement the Azure Key Vault provider.
4. Wire the resolver into API client construction.
5. Add `grokrs auth ...` diagnostics and extend `doctor`.
6. Update README, config examples, and operator docs.

## Acceptance Shape

- `grokrs` can authenticate via `XAI_API_KEY` or Azure Key Vault metadata.
- No secret value is written to config files, checked-in files, or local plaintext auth files.
- Operator diagnostics identify the auth source without exposing secret material.
- Existing env-based setups continue to work unchanged.

