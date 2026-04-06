# grokrs Product Spec

## Summary

`grokrs` is a safe Rust-only development CLI scaffold for Grok-oriented coding workflows.

The initial release is intentionally limited. It does not promise a production agent runtime yet. It establishes the boundaries needed to build one without inheriting unsafe defaults.

## Goals

- Provide a Rust workspace that encodes core safety concepts directly in types.
- Offer a small CLI that demonstrates config loading and effect evaluation.
- Establish repo-local docs, review artifacts, and bootstrap guidance.
- Prepare the repo for local `aivcs` and `sqry` registration.

## Non-Goals

- Full model orchestration
- Tool execution against the host shell
- Autonomous networked agent loops
- Dynamic plugin loading

## Functional Requirements

1. The CLI must load a TOML config from disk and summarize its safety posture.
2. The CLI must classify a small set of effects and evaluate them using a deny-by-default policy.
3. Workspace-relative paths must reject absolute or escaping input.
4. Session state must be typed by trust level.
5. The repository must include machine-readable implementation and review artifacts.

## Safety Requirements

1. Network access is denied by default.
2. Shell spawning is denied by default.
3. Workspace writes require validated relative paths.
4. Trust escalation must not be represented as a mutable boolean.

## Deliverables

- buildable Cargo workspace
- sample config
- initial CLI commands
- architecture and ops docs
- bootstrap review bundle

