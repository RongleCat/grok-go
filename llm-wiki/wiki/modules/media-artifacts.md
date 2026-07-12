# 模块：媒体产物

## 结论

所有 MCP/HTTP 图片与视频结果应落到 `~/.grok-go/artifacts/`，以绝对路径返回。这是 Codex 桌面内联渲染的关键约定。

## 职责文件

- `gateway/media_artifacts.rs`：下载、落盘、summary、MCP content 包装
- `gateway/image_bridge.rs`：Responses 内 image_gen function_call 服务端履行
- `gateway/job_affinity.rs`：视频异步 job 与账号绑定，便于 GET 轮询

## 核心函数

| 函数 | 作用 |
|---|---|
| `resolve_media_url` | 本地路径/file → data URL；https 透传 |
| `write_bytes` / `download_url_to_artifacts` | 写入 artifacts |
| `materialize_image_response` | 从上游 b64/url 抽出图片文件 |
| `materialize_video_response` / `poll_video_result` | 视频完成并下载 |
| `media_summary` | 统一 JSON：path/files/markdown |
| `mcp_media_content` | MCP 文本块包装 |

## 设计原则

- summary 中的 upstream 会剥离巨大 b64
- 优先让 Agent 只看见本地路径
- 视频 GET 必须打到创建 job 的同一账号（job_affinity）

## 相关页面

- [[mcp-tools]]
- [[gateway]]

## 来源

- `src-tauri/src/gateway/media_artifacts.rs`
- `src-tauri/src/gateway/image_bridge.rs`
- `src-tauri/src/gateway/job_affinity.rs`
