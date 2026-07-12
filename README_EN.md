<p align="center">
  <img src="assets/logo.png" alt="GrokGo logo" width="120" height="120" />
</p>

<h1 align="center">GrokGo</h1>

<p align="center"><strong>Local Grok gateway, ready out of the box</strong></p>
<p align="center"><em>Grok, ready to go for Codex</em></p>

<p align="center">
  <a href="./README.md">中文</a> ·
  <a href="./README_EN.md">English</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License" /></a>
  <a href="https://github.com/RongleCat/grok-go/stargazers"><img src="https://img.shields.io/github/stars/RongleCat/grok-go?style=social" alt="GitHub stars" /></a>
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey" alt="Platforms" />
  <img src="https://img.shields.io/badge/Tauri-2-orange" alt="Tauri 2" />
</p>

<p align="center">
  Follow the author on X
  <a href="https://x.com/cgnot996"><strong>@cgnot996</strong></a>
  · Repo
  <a href="https://github.com/RongleCat/grok-go">RongleCat/grok-go</a>
</p>

---

## Why GrokGo?

Connecting **Grok / xAI** to **Codex** or other AI tools usually means wiring OAuth, a local proxy, MCP, multi-account routing, and usage yourself.  
**GrokGo** packages that into a desktop gateway: launch, sign in, paste the endpoint, and go.

## Features

- **Responses + OpenAI-compatible APIs**: `/v1/responses`, `/v1/chat/completions`, `/v1/models`
- **MCP tools**: `x_search`, image generate/edit, video generate/edit
- **Multi-account OAuth**: host multiple accounts with weighted load balancing and auto refresh
- **Native media endpoints**: images/videos through the same authenticated gateway; artifacts under `~/.grok-go/artifacts/`
- **Usage visibility**: request logs, token totals, GitHub-style heatmap
- **Codex / CC Switch ready**: one-click inject for `mcp_servers.grok-go` and provider import
- **Optional LAN access** protected by a local bearer token

## Screenshots

| Overview | Accounts |
|:---:|:---:|
| ![Overview](assets/screenshots/overview.png) | ![Accounts](assets/screenshots/accounts.png) |

| Integrations | Usage |
|:---:|:---:|
| ![Integrations](assets/screenshots/integrations.png) | ![Usage](assets/screenshots/usage.png) |

## Quick start

### Download

Get installers from [Releases](https://github.com/RongleCat/grok-go/releases):

| Platform | Artifact |
|----------|----------|
| macOS Apple Silicon | `GrokGo_*_aarch64.dmg` |
| macOS Intel | `GrokGo_*_x64.dmg` |
| Windows x64 | `.msi` / `.exe` |

### Develop

```bash
pnpm install
pnpm tauri dev
```

Frontend only:

```bash
pnpm dev:ui
```

### Build

```bash
pnpm tauri build
```

See [docs/BUILD.md](./docs/BUILD.md) for cross-compile and release details.

## macOS: “damaged”, “unidentified developer”, or “cannot be opened”

Release builds are **not Apple-notarized** (requires a paid Developer ID). Gatekeeper often blocks the app after download — that is expected. Fixes:

### Option 1: Clear quarantine (recommended)

1. Drag `GrokGo.app` into Applications
2. Run in Terminal:

```bash
xattr -cr /Applications/GrokGo.app
# or wherever you put it:
# xattr -cr ~/Downloads/GrokGo.app
```

3. Open the app again

### Option 2: Right-click → Open

1. In Finder, **right-click** (or Control-click) `GrokGo.app`
2. Choose **Open**
3. Confirm **Open** in the dialog

> A normal double-click may still be blocked; use the context-menu Open once.

### Option 3: System Settings

1. **System Settings → Privacy & Security**
2. If you see a block message for GrokGo, click **Open Anyway**
3. Confirm with your password

### Still stuck?

```bash
xattr -l /Applications/GrokGo.app   # quarantine attribute should be gone
xattr -cr /Applications/GrokGo.app
open /Applications/GrokGo.app
```

Only download from this repo’s official [Releases](https://github.com/RongleCat/grok-go/releases). Once Apple signing + notarization are configured, these steps will no longer be needed.

## Connect Codex

1. Start GrokGo and copy from **Overview**:
   - Base URL: `http://127.0.0.1:<port>/v1`
   - Local Token
2. Point Codex at the Responses API with that base URL + bearer token
3. Optionally inject MCP from the **Integrations** page:

```toml
[mcp_servers.grok-go]
url = "http://127.0.0.1:<port>/mcp"

[mcp_servers.grok-go.http_headers]
Authorization = "Bearer <localToken>"
```

Preferred port is **8787** (auto-increments on conflict).

## Default endpoints

| Purpose | URL |
|---------|-----|
| Base | `http://127.0.0.1:<port>/v1` |
| Responses | `POST /v1/responses` |
| Chat Completions | `POST /v1/chat/completions` |
| Images | `POST /v1/images/generations`, `POST /v1/images/edits` |
| MCP | `http://127.0.0.1:<port>/mcp` |

## Config paths

```text
~/.grok-go/
  config.json
  auth.json
  data.db
  artifacts/
  backups/
```

## Stack

- Tauri 2 + Rust
- React + TypeScript + Vite
- Tailwind CSS

## Contributing

Issues and PRs are welcome — see [CONTRIBUTING.md](./CONTRIBUTING.md).  
Code of conduct: [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md)  
Security: [SECURITY.md](./SECURITY.md)

## License

[MIT](./LICENSE) © RongleCat

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=RongleCat/grok-go&type=Date)](https://star-history.com/#RongleCat/grok-go&Date)

---

<p align="center">
  If GrokGo helps you, star the repo and follow
  <a href="https://x.com/cgnot996">@cgnot996</a> on X
</p>
