# 架构总览

## 结论

GrokGo 是 **单进程桌面应用**：UI 通过 Tauri command 读写配置/账号/用量；同一进程内的 Axum 网关对外提供 `/v1/*` 与 `/mcp`；网关按策略挑选本地 OAuth 账号，转发到 `https://api.x.ai/v1`，并把媒体结果物化到 `~/.grok-go/artifacts/`。

## 进程内模块

```text
┌─────────────────────────────────────────────────────────────┐
│                        Tauri App                            │
│  React UI  ──invoke──► commands.rs ──► config/auth/usage    │
│       │                      │                              │
│       │                      ▼                              │
│       │               OAuthManager (auth.rs)                │
│       │                      │                              │
│       └──────────────► GatewayState                         │
│                              │                              │
│                     Axum server (gateway/*)                 │
│                     /health /v1/* /mcp                      │
└──────────────────────────────┬──────────────────────────────┘
                               │ Bearer local_token
                               ▼
                     Codex / 其他本地客户端
                               │
                               ▼ (upstream)
                     api.x.ai  +  media download
```

## 请求主路径（对话 / Responses）

1. 客户端 `POST /v1/responses`（或 chat/completions）带 `Authorization: Bearer <localToken>`
2. `proxy.authorize_request` 校验本地 token（`require_token=true` 时）
3. `resolve_model` 映射模型名
4. `sanitize_responses_request` 清洗 Codex 特有字段与 tool 类型
5. `optimize_responses_payload` 抑制多轮 base64/大文本膨胀；选号后可 Files 分流为 `file_id`
6. `send_with_account_failover` 选号 → ensure refresh → 上游发送；401/403/429/5xx 可换号
7. 若存在 image_gen 类 tool，服务端 tool loop 调 Imagine 并回填
8. 响应侧把 function_call 回写为 custom_tool_call（如需）；写 usage 日志
9. 返回 JSON 或 SSE

## 运行时目录

```text
~/.grok-go/
  config.json       AppConfig
  auth.json         账号与 token（unix 0600）
  data.db           请求日志
  artifacts/        媒体产物
  backups/          注入前配置备份
  agents-guide.md   版本化工具指引
  logs/             预留日志目录
```

## 默认网络

| 项 | 默认 |
|---|---|
| preferred_port | 8787（冲突递增，actual_port 记录实际端口） |
| bind_host | 127.0.0.1 |
| lan_enabled | false |
| require_token | true |
| xai_base_url | https://api.x.ai/v1 |
| oauth_redirect_port | 56121 |
| default_model | grok-4.5 |
| default_image_model | grok-imagine-image-quality |
| default_video_model | grok-imagine-video |

## 源码地图（按改动频率）

| 区域 | 路径 |
|---|---|
| 网关路由/MCP | `src-tauri/src/gateway/server.rs` |
| 代理/failover | `src-tauri/src/gateway/proxy.rs` |
| sanitize | `src-tauri/src/gateway/sanitize.rs` |
| payload 优化 / Files | `payload_optimize.rs` / `files_api.rs` |
| 媒体落盘 | `src-tauri/src/gateway/media_artifacts.rs` |
| 图生桥 | `src-tauri/src/gateway/image_bridge.rs` |
| 视频 job 亲和 | `src-tauri/src/gateway/job_affinity.rs` |
| OAuth | `src-tauri/src/auth.rs` |
| 选号 | `src-tauri/src/router.rs` |
| 集成 | `src-tauri/src/integrations.rs` |
| 配置 | `src-tauri/src/config.rs` |
| UI 页面 | `src/pages/*` |
| UI API | `src/lib/api.ts` |

## 相关页面

- [[project-overview]]
- [[../modules/gateway]]
- [[../modules/routing]]
- [[../concepts/request-sanitize]]

## 来源

- `src-tauri/src/lib.rs`
- `src-tauri/src/gateway/server.rs`
- `src-tauri/src/gateway/proxy.rs`
- `src-tauri/src/paths.rs`
- `src-tauri/src/config.rs`
