# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.9] - 2026-07-16

> 中英文对照 / Bilingual notes. English first (Keep a Changelog), then 中文摘要 under each section.
>
> **Highlight:** default **API** chat channel; Grok Build is opt-in with risk confirm; log tools + Codex agent true SSE recovery.

### Added

- **Settings → Channel**: segmented **API** | **Grok Build** for Codex / OpenAI / Claude chat routing (config field `experimentalImpersonateGrokBuild` kept for compatibility).
- **Grok Build confirm dialog**: switching to Grok Build requires explicit confirmation (account-restriction risk).
- **Request logs management**: retention policy (days / max rows), prune now, clear by range, clear all; TTFT (`first_token_ms`) and total latency columns.
- **Live log tail**: Logs page silently refreshes newest page; stable scroll when not pinned to top.
- **Codex agent true SSE**: stream deltas live; hold `response.completed` and multi-phase recover on premature stop (Build-style empty-completion recovery on session plane too when opted in).
- **Experimental build plane delivery** (R2 / O-series): optional SuperGrok session plane for non–Grok-Build clients; media always on console API; Anthropic thinking mode; gateway error envelopes; agents-guide refresh hooks.

### Fixed

- **Codex premature stop** on experimental build / tool turns: structure-based recovery + true SSE hold-completed path.
- **Non-stream first_token_ms**: record TTFT on non-stream request paths.
- **Multi-account rebalance**: strip `previous_response_id` when switching accounts so continuity does not pin a dead response id.

### Changed

- **Default chat channel is API** (`api.x.ai`). Grok Build / cli-chat-proxy for ordinary clients is **opt-in only**.
- README documents the channel risk and recommends API mode.
- Settings / wiki wording: API default; Build session plane no longer default-on.

**中文 · 新增**

- **设置 → 渠道选择**：API | Grok Build；切到 Grok Build 需二次确认（账号受限风险）。
- **日志管理**：保留策略、按天/区间清理、TTFT 与总耗时；日志页实时尾随。
- **Codex 真 SSE + 过早结束恢复**；可选 Build 会话面与 R2/O 系列网关增强。

**中文 · 修复**

- Build 面 / agent 工具回合过早结束；非流式首字耗时；换号剥 `previous_response_id`。

**中文 · 变更**

- **默认 API 渠道**（推荐）；Grok Build 仅 opt-in。README 写明限制并建议 API。

## [0.1.8] - 2026-07-16


> 中英文对照 / Bilingual notes. English first (Keep a Changelog), then 中文摘要 under each section.
>
> **Highlight:** multi-client inject (OpenCode / WorkBuddy / Cursor), gateway hardening, account enable≠health, agents-guide routing.

### Added

- **Integrations → Other clients**: one-click inject for **OpenCode** (provider + MCP), **WorkBuddy** (models + MCP), **Cursor** (MCP + BYOK Base URL / API Key / Model copy fields).
- **Accounts**: per-account config dialog (weight, reset health); enable/disable only toggles `enabled` (no longer writes `health=Disabled`).
- **CopyField** UI primitive: read-only `Input` + copy `Button` for endpoints/tokens across Overview and Integrations.
- **Agents guide routing rules** (generated `~/.grok-go/agents-guide.md` + Codex `AGENTS.md` short ref): prefer Codex native `imagegen`/`image_gen` for images; other enabled tools (esp. `x_search`, video) must use GrokGo MCP (`tools/list` then `tools/call`); forbid web_search / Chrome / twitter241 fallbacks unless health/MCP clearly fails.

### Fixed

- **Claude Code streams / 400s**: token-aware context budget for `/v1/messages`; SSE `message_stop` on upstream abort; oversized tool_call args stubbed as valid JSON; raw xAI error strings surfaced.
- **Codex TTFT**: default `emptyCompletionStreamBuffer=false` so `/v1/responses` streams tokens immediately.
- **Codex skills death loop**: Files offload only `function_call_output` blobs — no longer offloads `skills_instructions` into attachment_search stubs Codex cannot use.
- **CC Switch Codex import**: copy a GrokGo slot reusing the active `model_provider` id (keeps session history continuous).
- **SuperGrok quota cross-talk**: merge snapshots by `fetched_at`; sequential refresh with background maintainer queue.

### Changed

- **Integrations / Overview UI**: drop ad-hoc gray copy boxes; Grok Build tab uses standard list rows; MCP tool toggles use divide-y rows; concise UI copy rule documented in repo `AGENTS.md`.
- Account cards: remove static API rate badge; SuperGrok quota column wider.

**中文 · 新增**

- **其他客户端一键注入**：OpenCode / WorkBuddy / Cursor（Cursor 另提供 BYOK 字段复制）。
- **账号配置弹窗**：权重、重置健康；启用只改 `enabled`。
- **agents-guide 强制分流**：生图走 Codex 内置；搜索/视频走 GrokGo MCP；禁止随意降级。

**中文 · 修复**

- Claude Code 长会话裁剪与断流补全；Codex 真流式与 skills Files-offload 死循环；CC Switch 导入不丢会话；配额串号与串行刷新。

**中文 · 变更**

- 集成/概览控件改用标准 Input/Button；UI 文案与账号模型说明精简。

## [0.1.7] - 2026-07-14

> 中英文对照 / Bilingual notes. English first (Keep a Changelog), then 中文摘要 under each section.
>
> **Highlight:** Claude Code via local Anthropic Messages compatibility.

### Added

- **Claude Code / Anthropic Messages**: `POST /v1/messages` and `POST /v1/messages/count_tokens` convert to xAI Chat Completions (tools, streaming, model aliases). Set `ANTHROPIC_BASE_URL` to the local gateway root (**without** `/v1`).
- **Integrations → Claude Code**: base URL snippet, CC Switch Claude provider import (`app_type=claude`), MCP `enabled_claude` upsert.
- **Overview → Import to CC Switch**: choose **Codex** or **Claude Code** before writing the provider.
- **Grok Build session maintainer**: while routing is injected, periodically refresh a pool OAuth account, validate via OIDC **userinfo**, then write `~/.grok/auth.json` only if the IdP accepts the token (every ~15 minutes; first tick after ~45s).

### Fixed

- **CC Switch Claude import hijacking DeepSeek**: matching no longer uses bare `ANTHROPIC_BASE_URL`; UPDATE rewrites `website_url` / clears third-party icons so GrokGo is not a renamed DeepSeek row.
- **Grok Build open → browser login**: avoid overwriting `~/.grok/auth.json` with expired pool tokens; require refresh + userinfo success before write.
- **Window close UX**: with minimize-to-tray on, hide Dock/taskbar entry while keeping the tray icon; with it off, confirm quit (exiting stops the local gateway).

### Changed

- UI copy trimmed (less technical noise on Integrations / Settings / import dialogs).
- Account batch-import hint lists CPA and sub2api support.

**中文 · 新增**

- **Claude Code / Anthropic Messages 兼容**：本机 `ANTHROPIC_BASE_URL`（不要带 `/v1`）即可走账号池；流式与工具调用可用。
- **集成页 Claude Code**：片段复制、CC Switch Claude 应用导入。
- **概览一键导入 CC Switch**：可选 Codex 或 Claude Code。
- **Grok Build 会话定时维护**：注入路由后定期刷新 + userinfo 校验，确认有效再写 `auth.json`。

**中文 · 修复**

- **CC Switch 误改 DeepSeek**：严格匹配 GrokGo，完整重写配置字段。
- **开启 Grok Build 后跳网页授权**：禁止写入未校验/过期 token。
- **关窗行为**：托盘模式隐藏程序坞/任务栏图标；非托盘二次确认退出（代理随进程停止）。

**中文 · 变更**

- 文案精简；批量导入说明补充 CPA / sub2api。

## [0.1.6] - 2026-07-14

> 中英文对照 / Bilingual notes. English first (Keep a Changelog), then 中文摘要 under each section.

### Added

- **Grok Build multi-account routing (standard Session plane)**: Integrations can point Grok Build `cli_chat_proxy_base_url` at this gateway; pool OAuth sessions sync into `~/.grok/auth.json`; pre-route backup of `config.toml` + `auth.json` with one-click restore.
- **cli-chat-proxy build plane**: detect `X-XAI-Token-Auth` / `x-grok-model-override` / grok-build UA → upstream `cli-chat-proxy.grok.com` (SuperGrok credits, not console API billing).
- **Paywall remote routes**: proxy `GET /v1/user`, `/v1/settings`, `/v1/login-config`, `/v1/subagents/bundle` so GrowthBook `allow_access` works.
- **Codex premature-stop recovery**: structure-based empty/narration completion retry + synthetic shell probe so agent loops keep going (`empty_completion_retry`).

### Fixed

- **Prompt cache / token blow-up on Grok Build**: sticky keys honor `x-grok-conv-id` / `x-grok-session-id` / `x-grok-agent-id`; never overwrite client conv-id with a derived seed; build plane keeps `previous_response_id` / `prompt_cache_retention` / OpenAI-compat body fields; disables empty-completion silent retry, nuclear strip, and Files offload on the build plane (those are Codex/console-only).
- **cli-chat-proxy 426 Upgrade Required**: inject `x-grok-client-version` (default `0.2.101`) when the client omits it.
- **Build header passthrough**: forward `User-Agent`, `x-email`, `x-models-etag`, `Accept-Language`, tracing headers; inject a sensible UA when missing.
- **Codex empty completion / CC Switch import**: recover premature agent stops; tighten CC Switch model catalog import (thinking-depth models only when xAI accepts `reasoning.effort`).

### Changed

- **Account pick soft preference**: prefer JWT `referrer=grok-build` + full CLI scopes / higher tier for SuperGrok-capable pool accounts.
- **Integrations UI**: Grok Build panel shows protocol, account count, session email/tier/referrer, gate warnings, backup restore.

**中文 · 新增**

- **Grok Build 多账号路由（标准 Session）**：集成页一键写入 `cli_chat_proxy_base_url`；同步账号池到 `~/.grok/auth.json`；开启前备份并可一键还原。
- **cli-chat-proxy 原生平面**：识别 Build 客户端头 → 上游 SuperGrok 会话面（非 console API 计费）。
- **订阅门闸远程配置**：透传 `/v1/user`、`/v1/settings` 等，修复 subscription required。
- **Codex 过早结束恢复**：空完成/纯叙述结构判定 + 软重试 + 合成 shell 探针。

**中文 · 修复**

- **Build 多轮缓存/token**：会话黏连读官方 conv/session 头；保留原生连续性字段；Build 平面关闭 Codex 专用重试/核剥离/Files 分流。
- **426 版本门闸**：缺 `x-grok-client-version` 时注入默认版本。
- **Build 头透传**：补齐 UA / email / etag / 语言 / tracing。
- **Codex 空完成与 CC Switch 导入**收紧。

**中文 · 变更**

- 选号软偏好 `referrer=grok-build` 与更高 JWT tier。
- 集成页展示 Build 协议、会话与门闸告警、备份还原。

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

[Unreleased]: https://github.com/RongleCat/grok-go/compare/v0.1.8...HEAD
[0.1.8]: https://github.com/RongleCat/grok-go/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/RongleCat/grok-go/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/RongleCat/grok-go/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/RongleCat/grok-go/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/RongleCat/grok-go/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/RongleCat/grok-go/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/RongleCat/grok-go/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/RongleCat/grok-go/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/RongleCat/grok-go/releases/tag/v0.1.0
