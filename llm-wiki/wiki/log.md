# Wiki 日志

## 2026-07-15（WorkBuddy MCP 写入路径修正）

- UI「配置 MCP」编辑的是 `~/.workbuddy/mcp.json`，不是自动生成的 `.mcp.json`（connector-proxy）。
- 注入目标改为 `mcp.json`；type 用 `http`；顺带清理 `.mcp.json` 里残留的 grok-go。

## 2026-07-15（集成：其他客户端 + MCP 页布局）

- **其他客户端**标签：OpenCode（模型+MCP）、WorkBuddy（模型+MCP）、Cursor（MCP 注入 + BYOK 复制）。
- **MCP 工具**标签：左侧工具管理 · 右侧客户端 MCP 片段卡片（内部滚动）；原客户端复制卡片迁入此列。
- Cursor BYOK：Key/Base URL 在 secure storage，**不**做外部写入；只提供复制。
- 后端：`set_opencode_*` / `set_workbuddy_*` / `set_cursor_mcp_inject`，merge 写入、备份到 `~/.grok-go/backups/`。

## 2026-07-15（agents-guide 强制分流：图走 Codex / 其余走 GrokGo MCP）

- 模板：`integrations::agents_guide_file_body` + `agents_guide_ref_block`（勿只改 `~/.grok-go/agents-guide.md` 手写文件）。
- **图片**：优先 Codex 内置 `imagegen`/`image_gen`；MCP `image_*` 列为备选。
- **x_search / 视频等**：必须 GrokGo MCP（`/mcp` + Bearer），先 `tools/list` 再 `tools/call`。
- **禁止**因未注入 `mcp__grok-go__*` / 无原生 x_search / tool_search 失效就改用 web_search、Chrome、twitter241 或翻仓库猜参；仅 health/MCP 明确失败可降级并说明。

## 2026-07-15（Files offload 死循环：勿分流 skills）

- 会话 `019f65cb`：开场仅「嘿」却多次 `uploaded large blob` + `files-offload`。
- **根因**：`offload_large_text_blobs` 把 Codex 注入的 `skills_instructions`（~39KB）等 message 文本上传并换成 stub，再注入 “Use attachment_search / read them” → 模型去盘上找 `input-text-*.txt`、乱调工具，叠写 `write_stdin` 类型错误后进入多轮空转。
- **修复**：Files offload **只处理 tool output**；保护 bootstrap 文本检测；stub 改为中性「重跑工具取全文」，禁止 attachment_search 诱导。

## 2026-07-15（CC Switch 导入：复制槽 + 同 model_provider）

- Codex 会话绑 `model_provider` ID；写死 `grok-go` 会丢历史。
- **正确做法**：复制新增 GrokGo 槽（如 `GrokGo · sub2api`），**不覆盖**用户当前服务商配置；副本 TOML 使用与 `~/.codex/config.toml` **相同的** `model_provider` id。
- 再次导入只更新我们自己的副本行（notes/name 识别）。

## 2026-07-15（grok-4.5 经代理体感慢：SSE 整段缓冲）

- 数据：同模型 `responses` 平均 latency ~6.0s vs `grok-build` chat ~2.7s；responses 100k+ 上下文均值更高。
- **根因 1（体感主因）**：Codex 路径 `emptyCompletionRetry` 曾对**所有**流式 `/responses` 整段缓冲 SSE，首字时间≈整段生成；Grok Build 为 `build_plane` 真流式。
- **根因 2**：Codex 走 `api.x.ai` Responses，Build 走 `cli-chat-proxy` Chat；协议与上下文体积也不同。
- **修复**：新增 `emptyCompletionStreamBuffer` 默认 **false**——流式默认透传；仅显式开启才缓冲。

## 2026-07-15（账号配额：停止串扰 + 顺序刷新）

- 网关热路径 `patch_account_cache` 不再用请求开始时的旧 quota 覆盖新近刷新结果（按 `fetched_at` 合并）。
- 配额刷新串行、短超时；后台 silent 队列约 15 分钟一轮。

## 2026-07-15（Claude Code：400 upstream error）

- 会话 `6b1db793-…`：大 `Edit` 后下一轮 `/v1/messages` → 400；UI 只显示 `upstream error`。
- **显示层**：xAI 错误体是 `{"code":"…","error":"<string>"}`，旧映射只读 `/error/message` → 落到默认 `"upstream error"`。已改为解析 string `error` + 日志/DB 记 raw upstream。
- **根因（高概率）**：token 预算对 `tool_calls[].function.arguments` 做了**字符串中段截断**，破坏 JSON，xAI 整请求 400。已改为始终替换成**合法 JSON stub**（保留 key 预览）。
- 需重启 GrokGo；已失败会话可「继续」。

## 2026-07-14（Claude Code：Connection closed mid-response — 根因）

- 会话 `3cfe2d8d-…`：input 涨到 **~116k** 后连续 3 次流式中断；`request_logs` HTTP 200 但 **0 tokens**。
- **根因（不是“没补 message_stop”）**：Claude Code 每轮重放完整 tool 历史；原 `payload_optimize` 只有 **12–24MiB 字节预算**，文本 agent 环永远触达不到 → 不裁剪。大 prompt + 长 SSE 经 xAI/本地代理时中途被掐。
- **根治**：`enforce_chat_context_budget`（软 80k / 硬 100k 估算 token）在 `/v1/messages` 路径裁剪历史 tool/user、压缩 tools schema、按剩余窗口 cap `max_tokens`。
- **次要加固**（症状放大）：上游 SSE 读失败仍 `finish()` 发 `message_stop`，避免 Claude Code 只看到 Connection closed；0-token 流记 `error_summary`。
- 运维：重启 GrokGo；已毒化会话仍建议 `/compact` 或新开（客户端 transcript 仍大）。

## 2026-07-14（关窗：托盘隐藏 / 二次确认退出）

- **关闭时最小化到托盘 = 开**：关窗 `hide` + `skip_taskbar` + macOS `ActivationPolicy::Accessory`（去掉程序坞/任务栏图标，保留托盘）；托盘/菜单可 `Regular` 后再打开。
- **关闭时最小化到托盘 = 关**：关窗二次确认，确认后 `exit(0)` 停止进程与本地代理。
- 托盘左键切换显示/隐藏时同样走 hide/show 辅助函数。

## 2026-07-14（Grok Build auth.json 定时校验同步）

- 后台 maintainer（启动约 45s 后首跑，之后每 **15 分钟**）：仅在 `cli_chat_proxy` 已指向本机网关时执行。
- 流程：选池内最优 OAuth 账号 → `refresh_account` → **`GET userinfo` 探针**（auth.x.ai）→ 仅 `Valid` 才写入 `~/.grok/auth.json`；失败则不动现有文件。
- 开启集成时同一条路径（`require_success`）；最多试 3 个候选账号；写锁避免与定时任务竞态。

## 2026-07-14（Grok Build 开启后跳网页授权）

- 根因：`sync_grok_build_session_auth` 无条件用账号池覆盖 `~/.grok/auth.json`，且刷新失败时仍写入可能已过期的 access token；Grok 启动 silent refresh 失败就打开浏览器。
- 修复：强制 `refresh_account` + 近过期拒绝写入；`expires_at` 优先用 JWT exp；合并保留 profile 字段。
- 后续改为 userinfo 探针 + 定时 maintainer（见上条）。

## 2026-07-14（UI 文案精简）

- 全站中英文案去啰嗦与技术细节：概览导入弹窗、账号导入说明、集成 Claude/Grok Build、设置分流、备份分区说明等。
- 空字符串说明在页面侧条件渲染，避免空白行。

## 2026-07-14（概览页 CC Switch 导入二选一）

- 概览「同步到 CC Switch」改为「导入到 CC Switch」。
- 点击后弹窗选择 **Codex** 或 **Claude Code**，分别调用 `import_to_cc_switch` / `import_claude_to_cc_switch`。

## 2026-07-14（CC Switch Claude 导入误匹配 DeepSeek）

- 根因：`find_existing_grokgo_provider_for_app` 对 Claude 使用 `settings_config LIKE '%ANTHROPIC_BASE_URL%'`，几乎所有第三方 Claude provider（DeepSeek/Kimi/…）都会命中，导致 UPDATE 改名/改 notes 却留下对方 `website_url`/`icon`/env。
- 修复：严格身份（`name=GrokGo` / 我方 notes / Codex fingerprints / Claude 仅匹配本机网关 base）；UPDATE 时完整重写 `settings_config` + `website_url` 并清空 `icon`/`icon_color`；INSERT 写入本地 website_url。
- 回归测：`find_claude_provider_ignores_third_party_anthropic_env`。

## 2026-07-14（集成页 Claude Code + CC Switch）

- 集成页新增 **Claude Code** 选项卡：说明 `ANTHROPIC_BASE_URL`、可复制 env 片段。
- `import_claude_to_cc_switch`：写入 CC Switch `app_type=claude` 的 GrokGo provider（env 块）；无 DB 时导出 JSON。
- MCP upsert 支持 `enabled_claude`；与 Codex 导入分离，避免互相覆盖。

## 2026-07-14（Anthropic Messages 兼容落地）

- 实现自包含转换层 `gateway/anthropic/`（request/response/schema/stream），不引外部协议 crate。
- 路由：`POST /v1/messages`、`POST /v1/messages/count_tokens`；鉴权接受 `x-api-key` 与 Bearer。
- 上游走 xAI `/chat/completions` + 现有选号/failover/usage；工具调用双向保真 + 流式 `input_json_delta`。
- 细节对齐调研：tool_choice 映射、BatchTool 过滤、schema normalize、`disable_parallel_tool_use`。
- 单测 13 项；全库 `cargo test --lib` 138 通过。

## 2026-07-14（Anthropic / Claude Code 开源方案调研）

- 新建 `queries/anthropic-claude-code-research` + raw 协议笔记。
- 对比可复用开源：`llm-bridge-core`（Apache 纯协议库，首选）、`anthropic-proxy-rs`（MIT，Claude Code 向但 tool_choice 丢弃）、CLIProxyAPI / CCR（成熟网关，宜抄不宜嵌）、new-api（AGPL 勿嵌）。
- 明确自补项：`disable_parallel_tool_use`、schema normalize、x-api-key 鉴权、模型映射。

## 2026-07-14（token / 缓存命中护栏）

- 审计 Grok Build 路径：避免 Codex 专用逻辑破坏 SuperGrok 缓存与重复计费。
- `session_affinity` 识别 `x-grok-conv-id` / `x-grok-session-id` / `x-grok-agent-id`；优先透传客户端 `x-grok-conv-id`，不再用派生 seed 覆盖。
- Build 平面 `sanitize` 保留 `previous_response_id` / `prompt_cache_retention`；关闭 empty-completion 静默重试、nuclear strip、Files offload（file_id 属 console API）。
- Codex 失败重试在 strip 后重注入稳定 `prompt_cache_key`。
- Build 平面缺 `x-grok-client-version` 时注入默认 `0.2.101`（避免 cli-chat-proxy 426 Upgrade Required）。
- Build 透传补齐：`User-Agent` / `x-email` / `x-models-etag` / `Accept-Language` / tracing；缺 UA 时注入 `xai-grok-shell/<ver>`。
- Build sanitize 不再 strip `stream_options`/`safety_identifier`/`context_management`，也不注入 Codex 专用 tools。

## 2026-07-14（Grok Build 多账号原生路由）

- 集成页打开 Grok Build 标签：一键把 `cli_chat_proxy_base_url` 指到本机网关（SuperGrok 协议，非 API）。
- 开启前备份 `~/.grok/config.toml` + `auth.json`，支持一键还原。
- 网关识别 Grok Build plane，上游走 `cli-chat-proxy.grok.com`，透传 CLI 头 + session affinity。
- 明确不用 `models_base_url` / console API 计费路径。

## 2026-07-14（CC 导入思考深度）

- 实测 xAI 接受 `reasoning.effort` 的模型：`grok-4.5`、`grok-4.3`、`grok-4.20-multi-agent-0309`。
- CC Switch 导入：上述模型写入 `supported_reasoning_levels` / `default_reasoning_level`；默认模型另写 `model_reasoning_effort`。
- 固定深度变体（4.20 reasoning/non-reasoning、build）不挂深度字段，避免 Codex 发无效 effort。

## 2026-07-14（CC 导入仅保留 4.5 / 4.3）

- 用户实测 multi-agent / 4.20 / build 等在 Codex 侧不可用；导入 catalog 仅 `grok-4.5` + `grok-4.3`。
- 默认模型不在列表时钳制为 `grok-4.5`，仍带 `model_reasoning_effort`。

## 2026-07-14（reasoning-only 空完成恢复）

- 现象：Grok 返回仅 `reasoning` 的 `completed`，Codex `task_complete` 且无消息，任务中途停。
- 产品侧：`empty_completion` 检测 + `/v1/responses` 静默重试一次（流式缓冲 / 非流式 JSON）；配置 `emptyCompletionRetry` 默认 true。
- 文档：[[concepts/empty-completion-retry]]；更新 [[modules/gateway]]、[[modules/config-runtime]]。

## 2026-07-14（narration-only 提前结束）

- 现象：有一句「先对照…」类状态消息、无 tool call，Codex 仍 `task_complete`。
- 扩展：`is_narration_only_premature_stop` + 统一 `should_retry_premature_agent_stop`；tools 请求下短过渡话静默重试。
- 文档：更新 [[concepts/empty-completion-retry]]。

## 2026-07-14（合成 tool call 硬续跑）

- 实测：`tool_choice=required` / 软重试仍返回 narration（session `019f5eaf`），best-partial 无法推进循环。
- 最终兜底：`synthesize_forced_tool_response` 注入 `exec_command` 探测命令；软重试降为 1 次 + 固定 function tool_choice。
- 文档：更新 [[concepts/empty-completion-retry]]。

## 2026-07-14（通用化 premature 检测）

- 去掉中英「先/正在/let me」业务向词表；改为结构规则：tools + 无 tool_call + 非终态。
- 合成命令固定 `echo grok-go-continue`，不再从 input 抽路径 `ls`。
- 文档：更新 [[concepts/empty-completion-retry]]。

## 2026-07-14（CC 导入模型 / 日志 token / image_gen 入账）

- CC Switch 导入：`model_provider=grok-go`，modelCatalog 挂 xAI 全部文本模型（去掉 Composer / 图片模型）。
- Codex `image_gen` 桥接调用写入 request_logs（`image-gen-bridge`）。
- 日志来源/端点单元格上下排列；总量 token = input+output（input 已含 cache，不再加 cache）。

## 2026-07-14（CC Switch 同步 upsert + 人性化提示）

- 已有 GrokGo provider 时 UPDATE，并清理重复条目；首次才 INSERT。
- 成功/失败中文提示（含网关地址、MCP 状态）；Toast 支持多行与更长展示。

## 2026-07-14（UI 空状态 / 滚动 / 概览 Token）

- 统一 `EmptyState`（icon + 文案居中）；`PageShell`/`PageBody` 容器内滚动。
- Select 打开前同步定位，消除页面滚动条闪动。
- 概览「今日 Token」独立卡片：合计 + 入/出/缓存三栏。

## 2026-07-14（托盘 / SSO 导入）

- Windows 托盘：黑底白 logo 实心 PNG；设置页隐藏图标切换。
- 卡密导入：按 `eyJ…` JWT 形态匹配 SSO，支持 `邮箱|密码|SSO` 与带说明文字的卡商粘贴。

## 2026-07-13（token 异常 / 强制停止）

- 分支 `fix/token-consumption-and-force-stop`。
- 根因：Codex 多轮整包重放大文件/base64 图 → input token 与 body 线性膨胀；xAI 有图时不宜 store 历史。
- 方案对照 CPA / sub2api / xAI Files：`payload_optimize` 去重折叠截断；大文本 `POST /v1/files` → `file_id`；代理 `/v1/files`。
- 文档：[[concepts/payload-optimize]]；更新 [[modules/gateway]]。

## 2026-07-13（v0.1.4 发布准备）

- 批量导入 + SSO→OAuth Device Flow（`sso_convert.rs`）定稿；**移除** grok.com SSO 逆向（anti-bot）。
- Auth 写锁 + 按 id 合并，修复批量删除被异步额度写回复活。
- Windows OAuth：`rundll32` 打开完整授权 URL，修复 `Missing or invalid client_id`。
- 额度：SuperGrok 空账单兜底；刷新时探测 API rate-limit；UI 区分 API / SG。
- 日志页布局与统一 Select 组件；费用 `$` 前缀。

## 2026-07-13（账号批量导入 / 管理）

- 分支 `feat/account-batch-import-cpa-sub2api`。
- `account_import.rs`：CPA / sub2api RT / 卡密 SSO / GrokGo auth。
- `supports_image` / `supports_video`；批量导入/删/改 UI。
- 卡密 SSO 导入后 Device Flow 转 OAuth，网关只走 OAuth。

## 2026-07-12（hotfix Windows JSON）


- 分支 `hotfix/windows-json-config-load`：修复 Windows 下概览/账号页 `expected value at line 1 column 1`。
- 根因：`config.json`/`auth.json` 空文件、BOM 或损坏时 `serde_json::from_str` 硬失败，Tauri invoke 错误直出 UI。
- 处理：加载可恢复（备份坏文件 + 默认重建）+ 原子写盘；单测覆盖 empty/BOM/invalid/atomic overwrite。

## 2026-07-12

- 初始化项目 `llm-wiki`：建立 SCHEMA、raw 来源索引、核心 synthesis / modules / concepts / playbooks / queries。
- 目标：任意 Agent 接手都能从索引理解 GrokGo 是本地 Grok 网关，以及改代码应落在哪些模块。
- 基线版本：仓库 `0.1.1`。

## 2026-07-12（修复）

- 运行时 `agents-guide.md` 只渲染当前启用的 MCP 工具；与仓库开发用 `AGENTS.md` 隔离。
- 用量库空表 `SUM` 空值导致首次打开 Overview/Usage 失败：`COALESCE` + 查询/打开降级 + schema 先于 writer 初始化。
- CC Switch 导入在 MCP 已开启时写入 provider TOML 的 `mcp_servers.grok-go`，并 upsert `mcp_servers` 表。

## 2026-07-12（调研）

- 新建工作树分支 `feat/account-quota-usage`，调研账号剩余用量来源。
- 结论：截图 Weekly SuperGrok Limit 来自 `grok.com` `GrokBuildBilling/GetGrokCreditsConfig`；sub2api/GrokGo 现有路径只覆盖 `x-ratelimit-*`。
- 新增 [[concepts/account-quota]]、[[queries/account-quota-research]]、`raw/account-quota-sources.md`。
- 本机实测：Bearer OAuth 可拉到 percent/reset/product 拆分；reset 时区与截图一致。

## 2026-07-12（实现）

- 落地 SuperGrok 周配额：新增 `src-tauri/src/quota.rs`，调用 `GetGrokCreditsConfig` 解析剩余量与重置时间。
- Account 持久化 `quota` 字段；命令 `refresh_account_quota` / `refresh_all_account_quotas`。
- 账号页展示剩余 %、已用 %、进度条、重置时间、API/Grok Build 拆分，并支持单账号/全部刷新。

## 2026-07-12（UI）

- 账号卡片精简：三列固定布局（身份 / 用量 / 操作），高度压缩为约两行。
- 繁琐文案改为紧凑标签与图标按钮；完整说明通过 `title` hover 展示。
- 启用/权重/刷新/重登/删除改为无字或图标控件；冷却操作预留占位保持卡片对齐。

## 2026-07-12（调研·缓存与调度）

- 对照 sub2api / CPA：缓存与省量主路径是 **上游 prompt cache 粘连** + 失败冷却，不是本地整段回复缓存。
- 新增 [[queries/proxy-cache-routing-research]]：流量策略、可借鉴优先级与 GrokGo 差距。

## 2026-07-12（实现·稳定性调度）

- 落地 session affinity、quota-aware WRR、fill-first、prefer-soonest-reset、软并发上限。
- 401/403/429/5xx 分级冷却；失败时清 sticky；成功时 bind + 补全 `prompt_cache_key`。
- 设置 → 模型 页增加「流量分配」面板；默认对单号/多号无感增强稳定性。
- 模块：`session_affinity.rs`、`concurrency.rs`；`router.rs` / `proxy.rs` / `auth.rs` / `config.rs`。
- 完整答案语义缓存与 xAI reasoning replay 未做（复杂度高 / 体验风险，见调研文）。

## 2026-07-13（修复·prompt cache 全 0）

- 根因对照 sub2api/CPA：xAI 返回 `usage.input_tokens_details.cached_tokens`，GrokGo 只读 Anthropic 字段 → 统计恒 0。
- SSE 流式路径原先 `log_request(0,0,0)` 不解析 usage。
- 曾用 `previous_response_id` 注入 `prompt_cache_key`（每轮变化）破坏前缀缓存。
- 修复：多路径解析 cache、SSE 扫描 usage、稳定 cache key、`x-grok-conv-id`、response id 链式 sticky。

## 2026-07-13 — Drop SSO reverse; pure-Rust SSO→OAuth

- Removed grok.com reverse channel (`sso/`, `sso_dispatch`, wreq) after anti-bot 403.
- Card SSO import now runs OIDC Device Flow in Rust (`sso_convert.rs`): SSO cookie → access/refresh, then official OAuth gateway path.
- Added `convert_sso_accounts` for legacy SSO rows already on disk.
- UI: hide SSO pool; show convert button for legacy SSO.

## 2026-07-14 15:42 CST

- Grok Build 集成收敛为**标准 Session 路径**：`cli_chat_proxy_base_url` + 账号池 auth.json 同步；不做 API-key / models_base_url 模式。
- 开启前备份 `~/.grok/config.toml` 与 `auth.json`；UI 展示会话 email / JWT tier；网关侧 build plane 走 cli-chat-proxy。

## 2026-07-14 15:48 CST

- 定位 Grok Build 仍提示 subscription required：同步会话 JWT `referrer=sub2api`（非 `grok-build`），TUI 显示 x_premium_plus 仍 `allow_access=false`。
- 选号评分改为强优先 `referrer=grok-build` + 完整 cli scope；UI 展示 referrer 与门闸告警。

## 2026-07-14 15:55 CST

- Grok Build 仍显示 subscription required 根因：TUI 订阅门闸会请求 `GET /v1/user?include=subscription`，但 GrokGo 网关未实现该路由（404），GrowthBook 直接拦截。
- 修复：新增 `/v1/user` 透传到 cli-chat-proxy；选号对 JWT tier 做软偏好。实测上游返回 `subscriptionTier=GrokPro|XPremiumPlus`。

## 2026-07-14 16:00 CST

- Grok 日志确认：`/user` 已通且能识别 GrokPro/XPremiumPlus，但 GrowthBook `allow_access` 仍为 false（`paywall_check_gate_kept_allow_access_false`）。
- 修复：对 `GET /v1/user` 成功响应做门闸改写——身份对齐会话 JWT；付费档 `subscriptionTiers` 映射为 `SuperGrok`（`user-profile-gate-rewrite`）。

## 2026-07-14 16:03 CST

- 日志演进：把 `/user.subscriptionTiers` 改成 `SuperGrok` 后客户端变成 `paywall_check_no_subscription`（API 枚举不认该字符串）。
- 真正门闸字段在 **`GET /v1/settings`**：上游返回 `allow_access: true`、`subscription_tier_display: SuperGrok`；本地此前 404。
- 修复：透传 `/v1/settings`（及 login-config/subagents/bundle）；撤回错误的 SuperGrok 订阅枚举改写，仅保留 `/user` 身份对齐。

