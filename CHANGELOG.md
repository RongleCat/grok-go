# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.3] - 2026-07-13

### Added
- SuperGrok weekly credit quota on Accounts (remaining %, reset time, API / Grok Build breakdown) via `GetGrokCreditsConfig`
- Multi-account routing: session affinity, quota-aware weighted round-robin, fill-first, prefer-soonest-reset, soft per-account concurrency
- Settings → Models → Routing panel for strategy and affinity toggles
- Usage / Logs / Overview show **cache tokens** and cache hit rate

### Fixed
- Prompt cache accounting: parse xAI `input_tokens_details.cached_tokens`; scan SSE streams for usage (was always 0)
- Stable `prompt_cache_key` + `x-grok-conv-id` (do not inject rotating `previous_response_id`)
- Graded cooldowns for 401 / 403 / 429 / 5xx with sticky invalidation
- Windows: empty/corrupt/`UTF-8 BOM` `config.json` / `auth.json` no longer crash Overview & Accounts with `expected value at line 1 column 1`; bad files are backed up under `~/.grok-go/backups/` and recreated with defaults
- Config/auth writes use temp-file + rename (Windows-safe replace) to avoid truncated empty JSON after a crash mid-write

### Changed
- Accounts cards compacted (tags + icon actions); weight explained in page subtitle

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
