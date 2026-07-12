# Playbook：Agent 接手清单

## 结论

新 Agent 不要直接全库搜索瞎改。按下面顺序建立上下文，再动手。

## 步骤

1. 读 `llm-wiki/README.md`、`SCHEMA.md`、`wiki/index.md`
2. 读 [[../syntheses/project-overview]] 与 [[../syntheses/architecture]]
3. 确认版本三处一致：`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`
4. 确认本地运行时（如需联调）：
   - `curl -s http://127.0.0.1:8787/health`
   - 或 `pnpm tauri dev`
5. 定位任务类型 → 对应模块页（见 index）
6. 改代码后：
   - UI：`pnpm build:ui`
   - Rust：`cd src-tauri && cargo test --lib` / `cargo check`
7. 行为变化回写相关 wiki 页 + 追加 `wiki/log.md`
8. 禁止提交：`auth.json`、真实 token、`node_modules`、`target`、`dist`

## 改动边界提示

| 需求 | 优先文件 |
|---|---|
| 新 HTTP 端点 | `gateway/server.rs` + `proxy.rs` |
| MCP 新工具 | `server.rs` catalog + handle_tool_call + agents-guide 生成逻辑 |
| 登录问题 | `auth.rs`、`http_client.rs` 代理设置 |
| 限流/换号 | `router.rs`、`proxy.rs` failover |
| 一键集成坏了 | `integrations.rs` |
| 文案/页面 | `src/pages/*`、`src/i18n/*` |

## 相关页面

- [[../queries/faq]]
- [[dev-and-release]]
- [[debug-checklist]]
