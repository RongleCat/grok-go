# 模块：外部集成

## 结论

`integrations.rs` 负责把本机网关「接进」其他工具配置：Codex MCP、Codex AGENTS 指引、CC Switch provider、Grok Build CLI。改配置前会备份到 `~/.grok-go/backups/`。

## Codex MCP 注入

- 目标：`~/.codex/config.toml`（或 `$CODEX_HOME`）
- 写入：

```toml
[mcp_servers.grok-go]
url = "http://127.0.0.1:<port>/mcp"

[mcp_servers.grok-go.http_headers]
Authorization = "Bearer <localToken>"
```

- 兼容检测旧 id：`mcp_servers.grok-proxy`
- 关闭注入时会移除 MCP，并剥离 AGENTS 中的 guide 块

## Codex Agents Guide

- 完整指引写到：`~/.grok-go/agents-guide.md`（随软件维护，勿手改）
- `~/.codex/AGENTS.md` 只插一段带标记的短引用：
  - 标记：`<!-- grok-go:agents-guide:start -->` … `end`
  - 锚点：`grok-go:agents-guide-ref`
- 仓库根 `AGENTS.md` 也可能含同样标记（工作区级）

## CC Switch

- 目标 DB：`~/.cc-switch/cc-switch.db`
- `import_to_cc_switch` 写入 provider 记录，便于一键切换

## Grok Build / CLI

- 目标：`~/.grok/config.toml`（或 `$GROK_HOME`）
- 注入指向本网关的 base URL / 环境变量片段
- 清理历史错误的单模型 key `grok-go`

## 运行时 agents-guide

- 完整指引：`~/.grok-go/agents-guide.md`
- **只包含当前 `mcp_enabled_tools` 启用的工具**（改集成页 MCP 开关会重写）
- 与仓库开发用 `AGENTS.md` / `llm-wiki` 隔离

## CC Switch 导入

- 若 `auto_inject_codex_mcp` 为 true，或本机 `~/.codex/config.toml` 已注入 grok-go MCP：
  - provider 的 `config` TOML 会附带 `[mcp_servers.grok-go]`
  - 同时 upsert `mcp_servers` 表（`enabled_codex=1`）

## UI 入口

- 页面：`src/pages/Integrations.tsx`
- commands：`get_integrations`、`set_mcp_inject`、`inject_agents_guide`、`set_grok_build_inject`、`import_to_cc_switch`

## 相关页面

- [[mcp-tools]]
- [[config-runtime]]
- [[frontend-ui]]

## 来源

- `src-tauri/src/integrations.rs`
- `src-tauri/src/paths.rs`
