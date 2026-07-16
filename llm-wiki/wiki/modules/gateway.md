# 模块：Gateway

## 结论

本地 HTTP 网关在 `gateway/server.rs` 组装 Axum Router，真正转发与账号 failover 在 `gateway/proxy.rs`。MCP 与 REST 媒体接口共享同一鉴权与上游调用路径。

## HTTP 端点

| 方法 | 路径 | 作用 |
|---|---|---|
| GET | `/health` | 健康检查（`running` 等） |
| GET | `/v1/models` | 上游模型列表，失败则 curated 列表 |
| POST | `/v1/responses` | Responses API 主入口 |
| POST | `/v1/responses/compact` | 多轮 compaction |
| POST | `/v1/chat/completions` | Chat Completions 兼容 |
| POST | `/v1/messages` | **Anthropic Messages**（Claude Code；转 xAI chat + 回写） |
| POST | `/v1/messages/count_tokens` | Claude Code token 预检（粗估） |
| POST | `/v1/images/generations` | 文生图 |
| POST | `/v1/images/edits` | 图编辑 |
| POST | `/v1/videos/generations` | 文/图生视频 |
| POST | `/v1/videos/edits` | 视频编辑 |
| GET | `/v1/videos/{request_id}` | 视频 job 状态（账号亲和） |
| POST/GET | `/v1/files` | xAI Files 上传/列表（多轮大文件用 file_id） |
| GET/DELETE | `/v1/files/{file_id}` | 文件元数据 / 删除 |
| ANY | `/mcp` `/mcp/` | MCP JSON-RPC |

## 启动

- `lib.rs` setup 时 `start_gateway(GatewayState)` 异步启动
- 绑定 `preferred_port`，冲突时递增并写回 `actual_port`
- 状态：`GatewayState { running, proxy_ctx, ... }`

## 鉴权

- `require_token=true`（默认）时：`Authorization: Bearer <local_token>`
- token 来自 `config.json` 的 `localToken`，UI 可 rotate
- 局域网可开 `lan_enabled`，但仍建议 token 保护

## 关键实现点

- 授权：`proxy::authorize_request`（`Bearer` **或** `x-api-key` = `localToken`，兼容 Claude Code）
- 上游发送：`proxy::send_with_account_failover`（最多 3 账号尝试）
- 模型解析：`config::resolve_model`
- **Anthropic Messages**：`gateway/anthropic/*` + `proxy::proxy_anthropic_messages`（Messages ⇄ Chat Completions；流式 SSE 状态机）
- Responses 清洗：`sanitize::sanitize_responses_request`
- 多轮体积极膨胀抑制：`payload_optimize`（去重/折叠/截断 + `store:false`）
- 大文本 Files 分流：`files_api` + `offload_large_text_blobs`（`input_file.file_id`）
- image tool 服务端闭环：`proxy::run_image_gen_tool_loop` + `image_bridge`
- **reasoning-only 空完成恢复**：`empty_completion` + `proxy` 在返回客户端前静默重试一次（见 [[../concepts/empty-completion-retry]]）
- 视频 job 记住账号：`job_affinity`
- **双平面路由**：`gateway/build_plane_route.rs` 的 `decide_plane`（原生 Grok Build 标记 **或** 会话面开关，默认关 / API）

## 双平面：API 平面（默认）/ Grok Build 会话面（opt-in）

| 条件 | 推理上游 | 媒体上游 | 日志 `client_source` |
|---|---|---|---|
| 客户端带 Grok Build 标记 | `cli_chat_proxy_base_url` | `xai_base_url`（图片/视频） | `grok-build` |
| 开关 **关**（**默认** / API）+ 普通 Codex/OpenAI/Claude | `xai_base_url` | `xai_base_url` | 原协议标签 |
| `experimentalImpersonateGrokBuild=true`（opt-in）且无原生标记 | 同上（官方头：Token-Auth + authenticateresponse + client-mode/identifier + sampling `x-grok-*`） | 同上（cli-chat-proxy 无 media） | `experimental-build` / `experimental-build-media`（日志 id 保留） |

- 默认 **API**（`api.x.ai`）；设置页「渠道选择」分段器：**API** | **Grok Build**；切到 Grok Build 需二次确认（账号受限风险）。
- 配置字段名 `experimentalImpersonateGrokBuild` 仅兼容旧 config（true=Grok Build，false=API）。
- 会话面开启时：Responses 保留 continuity；Chat 剥 `service_tier` 等；Anthropic 仍回写 Messages 形态。
- empty-completion 恢复：console **与** 会话面仿冒（Codex agent loop）开；**仅原生 Grok Build TUI** 关。nuclear strip / files offload 仍仅 console。

## 相关页面

- [[../syntheses/architecture]]
- [[mcp-tools]]
- [[routing]]
- [[../concepts/request-sanitize]]
- [[../concepts/payload-optimize]]
- [[../concepts/empty-completion-retry]]
- [[media-artifacts]]

## 来源

- `src-tauri/src/gateway/server.rs`
- `src-tauri/src/gateway/proxy.rs`
- `src-tauri/src/gateway/build_plane_route.rs`
- `src-tauri/src/gateway/empty_completion.rs`
- `src-tauri/src/gateway/payload_optimize.rs`
- `src-tauri/src/gateway/files_api.rs`
