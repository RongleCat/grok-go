# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2026-07-12

### Fixed
- Fresh install / empty usage DB no longer breaks Overview, Usage, or Logs (NULL `SUM` on empty `request_logs`, safer open/init order, UI degrades to empty data)
- Usage schema is created before the async log writer starts; longer SQLite busy timeout and WAL setup for first-launch races

### Changed
- Runtime `~/.grok-go/agents-guide.md` only documents **currently enabled** MCP tools and stays separate from the in-repo project `AGENTS.md` / `llm-wiki`
- Changing MCP tool toggles rewrites the runtime agents guide; repo `AGENTS.md` is project-dev only
- CC Switch import includes `[mcp_servers.grok-go]` (and upserts the `mcp_servers` table) when Codex MCP inject is on or already present in `~/.codex/config.toml`

### Added
- Project `llm-wiki/` for agent handoff (architecture, modules, playbooks)
- README link for agents/contributors to the project wiki

## [0.1.1] - 2026-07-12

### Fixed
- Sticky OAuth account affinity for video job polls and image tool loops
- Release CI: do not pass empty Apple signing secrets

## [0.1.0] - 2026-07-12

### Added
- Initial public release as **GrokGo**
- Local Responses + OpenAI-compatible gateway
- MCP tools: `x_search`, image generation/edit, video generation/edit
- Multi-account OAuth hosting with weighted load balancing
- Model mapping for Codex model names
- Request logs, token totals, GitHub-style heatmap
- Codex MCP auto-inject/cleanup for `mcp_servers.grok-go`
- One-click CC Switch provider import
- Desktop app (Tauri 2) with tray support

### Changed
- Product renamed from Grok Proxy to GrokGo
- Config home moved to `~/.grok-go`
