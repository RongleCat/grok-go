# Wiki 日志

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

