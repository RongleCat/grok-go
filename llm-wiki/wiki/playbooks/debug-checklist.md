# Playbook：调试清单

## 网关起不来 / 端口不对

1. `curl -s http://127.0.0.1:8787/health`
2. 看 Overview 的 actual port（可能递增）
3. 查 `~/.grok-go/config.json` 的 `preferredPort`/`actualPort`
4. 是否被其他进程占用

## 401 invalid local bearer token

1. Overview 复制最新 Local Token
2. 客户端 `Authorization: Bearer ...` 是否匹配
3. 是否刚 rotate 过 token 但客户端未更新

## 无账号 / 登录失败

1. Accounts 是否完成 OAuth
2. 56121 回调端口是否被占
3. 系统/应用内 HTTP 代理是否需要开启（`http_proxy_enabled`）
4. 看账号 `last_upstream_error`、health、cooldown

## 全账号 cooldown

1. 多为 429 本地启发式，不是永久封禁
2. Accounts 可 clear cooldown
3. 等待 `cooldown_until` 或增加可用账号

## MCP 工具不可见

1. Integrations 是否注入 `mcp_servers.grok-go`
2. url 端口是否等于 actualPort
3. `mcp_enabled_tools` 是否过滤掉了目标工具
4. Codex 是否重启以重载 MCP

## 出图/视频没有本地路径

1. 检查 `~/.grok-go/artifacts/` 是否生成文件
2. Agent 是否错误展示了 CDN URL（应使用 path/markdown）
3. 视频轮询是否打到错误账号（job_affinity）

## Codex 工具调用报模型输入错误

1. 怀疑 sanitize 覆盖不全 → 看 `sanitize.rs` 与相关单测
2. 抓一份最小复现请求体（脱敏）再改转换逻辑

## Codex 读多文件/多图后 token 暴涨或任务强制停止

1. 现象：跑一段时间停、重启同一会话又停，goal 也救不了 → 多半是会话 history 已塞满 base64/大 tool 输出
2. 网关侧：`payload_optimize` 会去重/折叠历史图、截断大段 tool 输出、有图时 `store:false`；≥32KB 文本尝试 Files `file_id` 分流
3. 日志里搜 `payload optimized` / `files offload` / `uploaded large blob`
4. 客户端已毒化的线程：建议 **新开会话**；旧 transcript 本地仍大
5. 细节见 [[../concepts/payload-optimize]]

## Codex 刚开始探索就自己停了（无报错）

1. 现象 A：`task_complete`、无 assistant 正文，最后一轮只有 `reasoning`
2. 现象 B：有一句「先对照… / Let me…」状态话，但没有 tool call 就结束
3. 根因：上游 premature stop；Codex 不会自动续跑
4. 网关侧：`empty_completion_retry`（默认 true）对 empty + narration 静默重试一次；日志搜 `recovered premature agent stop` / `empty-completion-retry`
5. 若仍停：确认 `emptyCompletionRetry` 未关；可手动发「继续」
6. 细节见 [[../concepts/empty-completion-retry]]

## 概览/账号页：`expected value at line 1 column 1`

1. 这是 `config.json` / `auth.json` JSON 解析失败（常见：空文件、写一半崩溃、Windows 记事本 BOM）
2. 看 `~/.grok-go/backups/` 是否有 `*.bak`（应用会自动备份坏文件并重建默认）
3. 确认 `~/.grok-go/config.json` 与 `auth.json` 现在是合法 JSON（至少 `{}` / `{"accounts":[]}`）
4. 若备份里有 token，可手工合并回 `auth.json` 后重启
5. 仍失败：删空的 config/auth 让应用重建，或查日志目录 `~/.grok-go/logs/`

## 相关页面

- [[../modules/auth-oauth]]
- [[../modules/routing]]
- [[../modules/mcp-tools]]
- [[../concepts/request-sanitize]]
- [[../concepts/payload-optimize]]
- [[../concepts/empty-completion-retry]]
- [[../queries/faq]]
