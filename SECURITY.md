# Security

## What SlopBar can access

SlopBar reads the sign-in files that Claude Code and Codex already keep on your PC, in your Windows profile and in each WSL distro, to show your plan usage. It never refreshes these tokens and only sends them, over HTTPS, to the providers' own APIs: `api.anthropic.com` for Claude and `chatgpt.com` for Codex. Accounts you add are kept in SlopBar's data folder in your user profile (`%APPDATA%\dev.thelio.slopbar`).

Updates come from this repository's releases, and the app only installs one whose signature matches the key built into it.

## Reporting a vulnerability

Please report security issues privately through [GitHub's private vulnerability reporting](https://github.com/thelioo/slopbar/security/advisories/new) instead of opening a public issue. Fixes ship as regular releases, which installed copies pick up on their own.
