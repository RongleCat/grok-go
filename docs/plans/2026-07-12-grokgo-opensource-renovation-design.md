# GrokGo Open-Source Renovation Design

**Date:** 2026-07-12  
**Status:** Approved  
**Repo target:** `git@github.com:RongleCat/grok-go.git`

## Goal

将现有 Grok Proxy 桌面项目按开源标杆标准装修并全量品牌化为 **GrokGo**，提升可发现性、可信度与作者曝光（X / GitHub）。

## Brand

| Item | Value |
|------|--------|
| Display name | GrokGo |
| Package / crate / repo | `grok-go` |
| Lib crate | `grok_go_lib` |
| Bundle ID | `com.grokgo.desktop` |
| Config home | `~/.grok-go` |
| MCP server key | `mcp_servers.grok-go` |
| Locale storage key | `grok-go.locale` |
| Agents guide markers | `grok-go:agents-guide:*` |
| X | https://x.com/cgnot996 |
| GitHub | https://github.com/RongleCat/grok-go |

**Copy**

- 中文副标题：本地 Grok 网关，即开即用
- Slogan（中英共用）：`Grok, ready to go for Codex`
- English subtitle：Local Grok gateway, ready out of the box

**README header preview**

```text
# GrokGo
本地 Grok 网关，即开即用
Grok, ready to go for Codex
```

## Scope

### In scope

1. Full rename of all user-facing and package identifiers from Grok Proxy / grok-proxy to GrokGo / grok-go.
2. Standard open-source repo layout: MIT LICENSE, bilingual README, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, GitHub issue/PR templates, optional CI skeleton.
3. Brand assets: logo, app icons, tray icons, README screenshots.
4. Feature-oriented README with badges, X follow CTA, star history chart.
5. Headless browser screenshots of the frontend at **1080×720** (match Tauri window).
6. Local one-time machine migration: copy `~/.grok-proxy` → `~/.grok-go` if old dir exists.
7. Init git remote to `git@github.com:RongleCat/grok-go.git`.
8. Align `website/` naming/copy with GrokGo (no deploy required).

### Out of scope

- Runtime compatibility layer for old `~/.grok-proxy` path
- UI or docs migration notices for "old users"
- Core gateway feature work, OAuth end-to-end verification, dmg packaging fixes
- Website production deploy

## Local config migration (machine only)

- No application code paths for legacy `~/.grok-proxy`.
- No user-facing migration prompts.
- During implementation, on this machine:
  - If `~/.grok-proxy` exists and `~/.grok-go` does not: `cp -a ~/.grok-proxy ~/.grok-go` (or equivalent).
  - Leave old directory as-is unless explicitly cleaned later.
- App code only reads/writes `~/.grok-go`.

## Repository structure

```text
grok-go/
├── README.md
├── README_EN.md
├── LICENSE
├── CHANGELOG.md
├── CONTRIBUTING.md
├── CODE_OF_CONDUCT.md
├── SECURITY.md
├── AGENTS.md
├── .github/
│   ├── ISSUE_TEMPLATE/
│   │   ├── bug_report.yml
│   │   └── feature_request.yml
│   ├── PULL_REQUEST_TEMPLATE.md
│   └── workflows/ci.yml
├── assets/
│   ├── logo.png
│   └── screenshots/
├── docs/
├── src/
├── src-tauri/
├── website/
└── package.json
```

## README structure (ZH default, EN mirror)

1. Logo + name + 副标题 + slogan
2. Language switch links (`中文` / `English`)
3. Badges (MIT, stars, release/platform as available)
4. X follow CTA → https://x.com/cgnot996
5. Features
6. Screenshots (1080×720)
7. Quick start (dev / build)
8. Codex integration
9. Endpoints & default port
10. Config path `~/.grok-go`
11. Stack
12. Contributing / Security / License
13. Star history chart for `RongleCat/grok-go`
14. Footer author + X CTA again

## Screenshots

- Run frontend (`pnpm dev:ui` preferred; full `tauri dev` if needed for live status).
- Headless browser viewport **exactly 1080×720**.
- Capture at least: Overview, Accounts, Integrations, Usage (and Logs if useful).
- Store under `assets/screenshots/`.

## Rename map

| Old | New |
|-----|-----|
| Grok Proxy | GrokGo |
| grok-proxy | grok-go |
| grok_proxy / grok_proxy_lib | grok_go / grok_go_lib |
| com.grokproxy.desktop | com.grokgo.desktop |
| ~/.grok-proxy | ~/.grok-go |
| mcp_servers.grok-proxy | mcp_servers.grok-go |
| grok-proxy.locale | grok-go.locale |
| grok-proxy:agents-guide | grok-go:agents-guide |
| grok-proxy-site | grok-go-site |

### Primary files to touch

- `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`
- `src-tauri/src/**` (paths, integrations, UA strings, MCP keys, CC Switch names)
- `src/i18n/**`
- `README.md`, `AGENTS.md`, `docs/**`
- `website/app/**`, `website/package.json`
- Icons under `src-tauri/icons/` from new logo

## Logo & icons

- Source: user-provided app logo (black/white rounded-square mark with cloud + chevron/terminal + lightning arcs).
- Place canonical logo at `assets/logo.png`.
- Regenerate Tauri icon set + tray icons from logo.
- Sidebar brand mark uses logo instead of generic Activity icon.

## Open-source polish for exposure

- MIT license
- Bilingual README
- CONTRIBUTING / CODE_OF_CONDUCT / SECURITY / CHANGELOG
- Issue + PR templates
- CI skeleton: `pnpm build:ui` + `cargo check`
- Star history embed
- Clear X follow prompts
- Consistent GrokGo naming everywhere public-facing

## Success criteria

1. Fresh clone reads as a serious open-source desktop project named GrokGo.
2. No remaining user-facing "Grok Proxy" / `grok-proxy` product naming (internal historical plan docs may keep original filenames if needed; content should not confuse new visitors).
3. README ZH/EN complete with logo, screenshots, X link, star chart.
4. App/config/MCP identifiers use grok-go / GrokGo.
5. Local `~/.grok-go` exists with migrated config from this machine's old home.
6. Git remote points at `RongleCat/grok-go`.

## Approved decisions log

- Approach: full open-source package (A)
- Rename: full rename including package, bundle id, config dir, MCP key
- Migration: machine-only copy; no runtime compat; no prompts
- Screenshots: headless viewport = app window 1080×720
- Slogan: 中文副标题「本地 Grok 网关，即开即用」+ `Grok, ready to go for Codex`
- X: https://x.com/cgnot996
