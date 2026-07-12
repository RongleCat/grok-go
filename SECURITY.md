# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Reporting a Vulnerability

If you discover a security issue in GrokGo (for example token leakage, auth bypass, or unsafe local gateway behavior), please report it privately:

- Open a GitHub Security Advisory on [RongleCat/grok-go](https://github.com/RongleCat/grok-go), or
- Contact the maintainer on X: [@cgnot996](https://x.com/cgnot996)

Please include:
- A clear description of the issue
- Steps to reproduce
- Impact assessment if known

Do **not** open a public issue for sensitive vulnerabilities until a fix is available.

## Local security notes

- GrokGo stores OAuth tokens under `~/.grok-go/auth.json` — keep this directory private.
- The local gateway bearer token is shown in the app Overview page; treat it like a password on shared machines.
- Prefer binding to loopback unless you intentionally enable LAN access.
