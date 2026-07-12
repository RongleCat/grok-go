# 项目总览

## 结论

GrokGo 是 **本地运行的 Grok / xAI 网关桌面应用**。它把 OAuth 多账号、OpenAI 兼容 API、Responses API、MCP 媒体工具、用量统计和一键集成打包成开箱即用体验，让 Codex 等 Agent 直接调用 Grok，而不必各自处理鉴权与协议差异。

## 解决的问题

| 痛点 | GrokGo 做法 |
|---|---|
| xAI OAuth 与 token refresh 麻烦 | 桌面内一键登录，token 落在 `~/.grok-go/auth.json` 并自动刷新 |
| Codex 协议与 xAI 不完全一致 | 网关 sanitize / rewrite Responses 与 tool call |
| 多账号与限流 | 加权路由 + 本地 cooldown + 同请求 failover（最多 3 次） |
| 出图/出视频产物难用 | 下载到本地 artifacts，返回绝对路径 markdown |
| 每次手动改 Codex 配置 | 一键注入 `mcp_servers.grok-go` 与 agents-guide 引用 |

## 明确不做什么

- 不是云端 SaaS，不托管用户 token 到服务器。
- 不是通用 LLM 路由器（主线是 xAI / Grok）。
- 不替代 Codex 本体；只提供本地 provider + MCP 能力。
- Release 安装包当前默认未做 Apple 公证（需开发者账号）；Gatekeeper 提示属预期。

## 技术栈一句话

- **壳**：Tauri 2（托盘、开机启动、系统图标）
- **后端**：Rust + Axum + Tokio + rusqlite + reqwest
- **前端**：React 19 + TypeScript + Vite + Tailwind 4 + react-router
- **包管理**：pnpm；发布用 GitHub Actions 多平台矩阵

## 关键用户路径

1. 安装 / `pnpm tauri dev` 启动
2. Accounts 完成 xAI OAuth
3. Overview 复制 Base URL + Local Token
4. 在 Codex 配 Responses provider，或 Integrations 一键注入 MCP
5. Agent 调对话 / `x_search` / `image_gen` / `video_generate`

## 相关页面

- [[architecture]]
- [[../modules/gateway]]
- [[../playbooks/agent-onboarding]]

## 来源

- `README.md`
- `CHANGELOG.md`
- `src-tauri/src/lib.rs`
