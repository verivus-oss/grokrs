# Security Policy

## Supported Versions

`grokrs` is currently alpha software. Security fixes are applied on `main`.

## Reporting a Vulnerability

This repository is private and intended for internal development right now.

Do not file standard GitHub issues for security problems.

Report vulnerabilities through internal Verivus channels or directly to the repository administrators. Include:

- affected commit or branch
- reproduction steps
- impact assessment
- suggested containment if you have one

## Scope Notes

Security-sensitive areas in this repository include:

- policy evaluation and approval handling
- filesystem path validation
- shell and network execution gates
- credential handling and transport auth
- SQLite state, approvals, and transcript storage
- MCP tool integration

