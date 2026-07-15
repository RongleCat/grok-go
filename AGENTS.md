# GrokGo — 项目开发 AGENTS.md

> 本文件是**仓库内**给开发 Agent 的说明，与运行时注入到用户 `~/.codex/AGENTS.md` / `~/.grok-go/agents-guide.md` 的 MCP 工具指引**隔离**。
>
> - 运行时工具指引：由应用生成 `~/.grok-go/agents-guide.md`（只含当前启用的 MCP 工具）
> - 项目理解：见 [`llm-wiki/`](./llm-wiki/)

## 项目一句话

GrokGo 是本地 Grok / xAI 网关桌面应用（Tauri 2 + React + Rust）：多账号 OAuth、OpenAI/Responses 兼容 API、MCP 媒体工具、用量统计、一键集成 Codex / CC Switch。

## 接手入口

1. [`llm-wiki/README.md`](./llm-wiki/README.md)
2. [`llm-wiki/wiki/index.md`](./llm-wiki/wiki/index.md)
3. [`llm-wiki/wiki/playbooks/agent-onboarding.md`](./llm-wiki/wiki/playbooks/agent-onboarding.md)

## 开发命令

```bash
pnpm install
pnpm tauri dev
pnpm build:ui
cd src-tauri && cargo check && cargo test --lib
```

## 关键约定

- 配置目录：`~/.grok-go/`（不要把 `auth.json` / token 提交进仓库）
- MCP server id：`grok-go`（兼容旧 id `grok-proxy`）
- 运行时 agents-guide 只反映 **当前启用的 MCP 工具**；改 `mcp_enabled_tools` 后应用会重写 `~/.grok-go/agents-guide.md`
- 仓库本文件不要粘贴完整工具参数表；工具参数以网关 `tools/list` 与运行时 guide 为准
- 代码行为变更后：回写 `llm-wiki/wiki/**` 并追加 `llm-wiki/wiki/log.md`

## UI 文案（全应用）

**对整个产品 UI 生效**（`src/pages/**`、`src/components/**`、`src/i18n/**`），不限账号页。改任何界面时都要过一遍：能删的说明就删。

### 原则

1. **控件能自解释的，不写副标题/Desc/Hint。**
2. **只在反直觉处加半句**（例：权重「越大分到越多」；并发「0 = 不限制」）。
3. **禁止**在页面上写实现细节：`enabled` vs `health`、选号算法、failover、token 路径、协议字段等 → 写本文件或 `llm-wiki`。
4. Dialog / 设置行：标题 + 控件即可；长 `description` / 灰字段落默认不要。
5. 中英文同步精简；toast/错误可稍完整，仍避免说明书腔。
6. **改完 UI 后自检**：是否出现大段说明、是否重复页面标题、是否只有开发者才懂的词。

### 反例 → 正例

| 反例 | 正例 |
|------|------|
| 「启用/禁用只改是否参与选号，与健康无关」 | （不写；写进下方账号模型） |
| 「关闭时隐藏到托盘并去掉程序坞图标；关闭此项则…」 | 「关窗进托盘」 |
| 弹窗顶部整段架构说明 | 无 description |

## 账号模型（实现用）

| 字段 | 含义 |
|------|------|
| `enabled` | 是否参与选号。启用/禁用**只改这个**，不改 `health`。 |
| `weight` | 选号权重，**越大分到越多**。 |
| `health` | 运行态：healthy / degraded / cooldown。冷却中会暂不选。 |
| 重置健康 | 清冷却、失败计数、错误摘要 → healthy；**不改** `enabled`。 |

- 卡片勾选 = 批量操作对象；「启用」开关 = `enabled`。
- 全局软并发：`account_max_concurrency`；会话粘连：`session_affinity`。

## 源码速查

| 区域 | 路径 |
|------|------|
| 网关 / MCP | `src-tauri/src/gateway/` |
| OAuth | `src-tauri/src/auth.rs` |
| 选号 | `src-tauri/src/router.rs` |
| 集成注入 | `src-tauri/src/integrations.rs` |
| 用量 DB | `src-tauri/src/usage.rs` |
| UI | `src/pages/`、`src/lib/api.ts` |

## 与运行时注入的关系

应用在用户机器上可能写入：

- `~/.codex/config.toml` → `[mcp_servers.grok-go]`
- `~/.codex/AGENTS.md` → **短引用**（指向 `~/.grok-go/agents-guide.md`，不是本仓库文件）
- `~/.grok-go/agents-guide.md` → 启用工具的精简参数说明

请勿把上述运行时全文同步回本仓库的 `AGENTS.md`。
