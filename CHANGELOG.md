# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.5] - 2026-07-14

> 中英文对照 / Bilingual notes. English first (Keep a Changelog), then 中文摘要 under each section.

### Fixed

- **Token blow-up / forced stop on multi-file Codex turns**: gateway optimizes Responses/chat payloads before upstream (image dedupe & historical collapse, large tool-output truncation, `store:false` when images are present) and offloads ≥32KB text blobs to the xAI Files API as `file_id` references. Also proxies `/v1/files` for explicit upload-by-id flows.
- **Windows tray all-white**: solid black-background white-logo 32px tray asset (not transparent white glyph).
- **SSO card import**: detect `eyJ…` JWT by shape (not fixed `----` layout); supports `email|password|SSO` and noisy seller pastes.
- **Select dropdown scrollbar flash**: measure panel position before mount; list scroll stays inside the menu.
- **Logs virtual list after refresh**: reset scroll offset so rows/scrollbar stay correct.
- **Heatmap tooltip**: arrow aligns with the clicked cell; switch cells in one click; blank click still closes.

### Changed

- **Windows settings**: hide app icon dark/light switch (tray fixed to black-bg brand).
- **UI empty states**: centered icon + copy (`EmptyState`) for accounts / logs / mapping.
- **Page scroll**: lists scroll inside containers (`PageShell` / `PageBody`), not the whole app shell.
- **Overview layout**: metrics row flex `1 1 1 3`; endpoints put API + MCP on one row, token full-width; token card shows total + in/out/cache.
- **Logs table**: drop “recent requests” title; source/endpoint inline; wider latency column.
- **Heatmap**: graph + legend horizontally centered.

**中文 · 修复**

- **读多文件/多图后 token 暴涨、任务被强制停止**：上行 payload 优化 + Files `file_id` 分流 + `/v1/files` 代理。
- **Windows 托盘全白**：黑底白 logo 实心小图标。
- **卡密 SSO 导入**：按 JWT 形态识别，支持 `|` 等任意分隔。
- **下拉框打开闪滚动条**：打开前同步定位。
- **日志刷新后虚拟列表错乱**：重置滚动位置。
- **热力图提示框**：箭头对齐格子、可直接点另一格切换、空白关闭。

**中文 · 变更**

- **Windows 设置**：隐藏图标明暗切换。
- **空状态**：统一居中 icon + 文案。
- **页面滚动**：列表在容器内滚动。
- **概览 / 日志 / 热力图** 布局与表格细节调整（见上）。

## [0.1.4] - 2026-07-13

> 中英文对照 / Bilingual notes. English first (Keep a Changelog), then 中文摘要 under each section.

### Added

- **Batch account import** for CPA `xai-*.json`, sub2api multi-line refresh tokens, GrokGo `auth.json`, and card lines `email----password----SSO` (web SSO JWT).
- **SSO → OAuth Device Flow** (pure Rust): card SSO cookies are converted via `auth.x.ai` device authorization, then accounts use the existing OAuth gateway path (no grok.com reverse proxy).
- **Batch account ops**: multi-select enable/disable, media flags, weight, cooldown clear, verified multi-delete with disk persistence check.
- **Per-account media capability**: `supportsImage` / `supportsVideo` filters routing for image/video jobs.
- **Custom `Select` component** (portal dropdown) for consistent cross-platform UI (Accounts filter, Settings models/routing, Mapping).
- **Logs**: routed account label, denser layout (account/time + status tag, source/endpoint stack, wider Token column).
- **Quota refresh probe**: manual refresh also hits `GET {xai_base}/models` to refresh API `x-ratelimit-*` headers alongside SuperGrok weekly credits.

**中文 · 新增**

- **批量导入账号**：CPA `xai-*.json`、sub2api 多行 RT、GrokGo `auth.json`、卡密 `邮箱----密码----SSO`。
- **SSO→OAuth（纯 Rust Device Flow）**：卡密 SSO 经 `auth.x.ai` 设备授权换成 access/refresh，之后走现有 OAuth 网关（已移除 grok.com 逆向反代）。
- **批量操作**：多选启用/禁用、图/视能力、权重、清冷却、删除后写盘校验。
- **账号媒体能力**：`supportsImage` / `supportsVideo` 参与图/视频选号。
- **统一 Select 组件**：自绘下拉，账号筛选 / 设置模型与路由 / 模型映射跨端一致。
- **日志页**：展示路由命中账号；账号/时间+状态标签、来源/端点合并；Token 列加宽。
- **刷新用量**：在 SuperGrok 周额度之外，额外探测 API `x-ratelimit-*`。

### Fixed

- **Windows OAuth login**: `cmd /C start` split OAuth URLs on `&`, so browsers opened without `client_id` → `Missing or invalid client_id`. Now uses `rundll32 url.dll,FileProtocolHandler` (with quoted `cmd` / PowerShell fallbacks).
- **Empty OAuth `client_id`**: config defaults + `effective_xai_client_id()` + refuse to persist empty client id on settings save/import.
- **Batch delete not durable**: async quota/token writers re-saved stale full account lists after delete. Auth writes are locked; post-await updates merge into the **live** store and never re-insert deleted ids; delete re-reads disk to verify.
- **Batch delete UI no-op**: replace `window.confirm` (often broken in WKWebView) with in-app `ConfirmDialog`.
- **Unused SuperGrok payload**: accounts with no `used%` / empty products no longer error as `could not parse quota percent`; default to 0% used / 100% remaining and show API rate-limit tags clearly.
- **Quota refresh clobbering concurrent rate limits**: merge prefers fresher live rate-limit / success markers when applying snapshots after `await`.

**中文 · 修复**

- **Windows 授权登录**：`cmd start` 未引用 URL，`&` 截断 query，浏览器丢失 `client_id`。改为 `rundll32` 打开完整链接。
- **空 client_id**：配置默认值与运行时回落，禁止把空 client_id 写入配置。
- **批量删除“假删”**：异步额度/token 写回旧整表。现已加锁、按 id 合并、删除后校验磁盘。
- **批量删除无响应**：去掉 `window.confirm`，改用应用内确认框。
- **未使用 SuperGrok 号**：空账单响应不再报解析错误；默认 0%/100%，并突出 API 限额标签。
- **刷新额度覆盖并发状态**：合并写入时保留更新的 rate-limit / 成功时间戳。

### Changed

- Removed the experimental **grok.com SSO reverse** channel (`sso/*`, `sso_dispatch`, browser-impersonation deps). Card SSO is import-time conversion only.
- Accounts UI: SuperGrok (`SG`) vs API rate-limit (`API n/n`) dual display; post-import auto quota refresh; import/convert status messaging.
- Cost formatting uses `$x.xxxx` without locale `US$` prefix.
- Logs cost / layout / account routing display as above.

**中文 · 变更**

- 移除 **grok.com SSO 逆向** 通道；卡密仅在导入时转为 OAuth。
- 账号页区分 SuperGrok 周额度与 API 请求限额；导入成功自动刷额度。
- 费用显示为 `$` 前缀（无 `US$`）。
- 日志布局与命中账号展示见上。

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

[Unreleased]: https://github.com/RongleCat/grok-go/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/RongleCat/grok-go/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/RongleCat/grok-go/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/RongleCat/grok-go/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/RongleCat/grok-go/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/RongleCat/grok-go/releases/tag/v0.1.0
