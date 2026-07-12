# Wiki 日志

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
