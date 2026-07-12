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

## 相关页面

- [[../modules/auth-oauth]]
- [[../modules/routing]]
- [[../modules/mcp-tools]]
- [[../concepts/request-sanitize]]
- [[../queries/faq]]
