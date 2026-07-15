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
- `import_to_cc_switch` → `app_type=codex`（Responses / OpenAI 兼容）
- `import_claude_to_cc_switch` → `app_type=claude`（Anthropic Messages env）

### Claude Code provider 形状

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:<port>",
    "ANTHROPIC_AUTH_TOKEN": "<localToken>",
    "ANTHROPIC_API_KEY": "<localToken>",
    "ANTHROPIC_MODEL": "grok-4.5",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "grok-4.3",
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "grok-4.5",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "grok-4.5"
  }
}
```

- `ANTHROPIC_BASE_URL` **不含** `/v1`（Claude Code 会拼 `/v1/messages`）
- 网关侧由 `gateway/anthropic` 做 Messages ⇄ Chat Completions
- MCP：同步时对 `mcp_servers.grok-go` 置 `enabled_claude=1`（列存在时）

## Grok Build / CLI（标准 Session 路由）

- 目标：`~/.grok/config.toml` + `~/.grok/auth.json`（`$GROK_HOME`）
- **只支持标准 SuperGrok 路径**（不接 Custom Models / API-key 摸索）：
  - 写入 `[endpoints] cli_chat_proxy_base_url = "http://127.0.0.1:<port>/v1"`
  - 等价 env：`GROK_CLI_CHAT_PROXY_BASE_URL`
  - 开启时把账号池中较优 OAuth 会话同步进 `auth.json`（绕过免费号订阅门闸）
  - **禁止**写入 `models_base_url`（console API / 计费路径）；若历史误指向本机则清理
- 网关侧：识别 `X-XAI-Token-Auth` / `x-grok-model-override` / grok-build UA → 上游 `cli_chat_proxy_base_url`（默认 `https://cli-chat-proxy.grok.com/v1`）
  - 替换客户端 Bearer 为池账号 token；透传 build 头 + session affinity / `prompt_cache_key`
- 开启前备份到 `~/.grok-go/backups/grok-build-pre-route/`；支持一键还原
- 清理历史错误：指向本机的 `models_base_url`、单模型 key `grok-go`

## 运行时 agents-guide

- 完整指引：`~/.grok-go/agents-guide.md`
- **只包含当前 `mcp_enabled_tools` 启用的工具**（改集成页 MCP 开关会重写）
- 与仓库开发用 `AGENTS.md` / `llm-wiki` 隔离

## CC Switch 导入

- **Codex 导入（复制槽，防会话丢失）**：
  1. **不**改写用户当前 `is_current` 原服务商配置；
  2. **复制新增**（或更新）一条 GrokGo 副本（显示名如 `GrokGo · sub2api`）；
  3. 副本 TOML 的 `model_provider` **与当前 `~/.codex/config.toml` 相同（会话 `session_meta` 仍对得上）；
  4. 无当前 provider 时回退 `GrokGo` / `grok-go`。
- Claude 导入仍用独立 `GrokGo` 名（会话体系不同）。
- 若 `auto_inject_codex_mcp` 为 true，或本机 `~/.codex/config.toml` 已注入 grok-go MCP：
  - provider 的 `config` TOML 会附带 `[mcp_servers.grok-go]`
  - 同时 upsert `mcp_servers` 表（`enabled_codex=1`）
- `modelCatalog` **仅**挂实测可用模型：`grok-4.5`、`grok-4.3`（含思考深度字段）
  - `grok-4.5`：`low|medium|high`
  - `grok-4.3`：额外含 `none`
  - 其余 xAI 文本 id（4.20 固定变体 / multi-agent / build）不进导入列表
- provider `config` TOML：`model` 钳制到上述列表，并含 `model_reasoning_effort = "medium"`

## UI 入口

- 页面：`src/pages/Integrations.tsx`
- commands：`get_integrations`、`set_mcp_inject`、`inject_agents_guide`、`set_grok_build_inject`、`restore_grok_build_backup`、`import_to_cc_switch`

## 相关页面

- [[mcp-tools]]
- [[config-runtime]]
- [[frontend-ui]]

## 来源

- `src-tauri/src/integrations.rs`
- `src-tauri/src/paths.rs`
