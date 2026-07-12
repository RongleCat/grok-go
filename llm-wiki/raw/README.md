# raw 来源索引

本目录只放「来源指针」和必要的不可变摘录，不放可编译知识结论。  
知识结论写在 `wiki/`。

## 仓库内权威来源

| 来源 | 路径 | 用途 |
|---|---|---|
| 中文 README | `../../README.md` | 产品定位、安装、端点表 |
| 英文 README | `../../README_EN.md` | 同上（英文） |
| 构建说明 | `../../docs/BUILD.md` | 本地构建与 CI 发布 |
| 变更日志 | `../../CHANGELOG.md` | 版本历史 |
| 前端入口 | `../../src/App.tsx` | 页面路由 |
| 前端 API 层 | `../../src/lib/api.ts` | Tauri command 封装 |
| Rust 入口 | `../../src-tauri/src/lib.rs` | 托盘、gateway 启动、command 注册 |
| 网关路由 | `../../src-tauri/src/gateway/server.rs` | HTTP 路由与 MCP |
| 代理核心 | `../../src-tauri/src/gateway/proxy.rs` | 上游转发、failover、image loop |
| 请求清洗 | `../../src-tauri/src/gateway/sanitize.rs` | Codex → xAI 兼容 |
| OAuth | `../../src-tauri/src/auth.rs` | xAI 登录 |
| 账号路由 | `../../src-tauri/src/router.rs` | 选号策略 |
| 集成注入 | `../../src-tauri/src/integrations.rs` | Codex / CC Switch / Grok Build |
| 配置模型 | `../../src-tauri/src/config.rs` | AppConfig / Account / resolve_model |
| 路径约定 | `../../src-tauri/src/paths.rs` | `~/.grok-go` 与外部工具路径 |

## 运行时来源（机器本地，勿入库）

| 路径 | 内容 |
|---|---|
| `~/.grok-go/config.json` | 端口、token、模型映射、路由策略 |
| `~/.grok-go/auth.json` | 账号 token（敏感，0600） |
| `~/.grok-go/data.db` | 请求日志 SQLite |
| `~/.grok-go/artifacts/` | 生成图片/视频本地产物 |
| `~/.grok-go/agents-guide.md` | 注入给 Codex 的工具指引 |
| `~/.codex/config.toml` | MCP 注入目标 |
| `~/.codex/AGENTS.md` | agents-guide 短引用目标 |
| `~/.cc-switch/cc-switch.db` | CC Switch provider 导入目标 |
| `~/.grok/config.toml` | Grok Build / CLI 注入目标 |

## 收录规则

- 需要长期保留的调研 PDF / 截图可放 `raw/assets/`，并在 wiki 页标注来源。
- 不要把真实 `auth.json` 或 token 复制进 raw。
