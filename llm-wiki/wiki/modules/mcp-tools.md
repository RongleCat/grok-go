# 模块：MCP 工具

## 结论

MCP 挂在 `http://127.0.0.1:<port>/mcp`，需 Bearer local token。工具目录在 `gateway/server.rs` 的 `mcp_tools_catalog_all()`，**参数以运行时 `tools/list` 为准**；`~/.grok-go/agents-guide.md` 是给人/Agent 读的同步摘要（由 `integrations::agents_guide_file_body` 生成）。

## Codex 强制分流（agents-guide）

| 能力 | 优先路径 | 备注 |
|---|---|---|
| 文生图 / 图编辑 | Codex 内置 `imagegen` / `image_gen` | **不要**默认走 GrokGo MCP `image_*` |
| `x_search`、`video_generate`、`video_edit` 等非图片 | GrokGo MCP | 先 `tools/list` 再 `tools/call`；禁止 web_search / Chrome / twitter241 顶替 |
| 降级 | 仅 `/health` 或 MCP 明确失败 | 必须说明原因 |

## 工具清单

| 工具 | 作用 | 必填 | agents-guide 归类 |
|---|---|---|---|
| `x_search` | 搜 X/Twitter | `query` | 优先 MCP |
| `image_gen` | 文生图 | `prompt` | MCP 备选（非默认） |
| `image_generate` | `image_gen` 别名 | `prompt` | MCP 备选（非默认） |
| `image_edit` | 图编辑 | `prompt`, `image_url` | MCP 备选（非默认） |
| `video_generate` | 文生/图生/多图参考视频 | `prompt`（+ 可选 image 字段） | 优先 MCP |
| `video_edit` | 视频编辑 | `prompt`, `video_url` | 优先 MCP |

## 开关

- `AppConfig.mcp_enabled_tools`：
  - `null`：全部启用（默认）
  - 数组：只暴露列出的工具名
- UI Settings 可配置；`tools/list` 与 `tools/call` 都尊重过滤

## 调用链路（媒体类）

1. MCP `tools/call`
2. 组装上游 body（图片/视频模型默认值）
3. `call_upstream` → 同网关鉴权与账号 failover
4. `materialize_*_response` 下载到 artifacts
5. `mcp_media_content` 返回 JSON 文本 + markdown 提示

## 媒体输入约定

- 接受：`https://`、`data:`、本地绝对路径、`file://`
- 本地路径会转 data URL 再给上游（`media_artifacts::resolve_media_url`）

## 返回约定（对 Agent 强制）

- 使用 `path` / `files` / `markdown` 中的 **绝对本地路径**
- 用 `![image](/abs/path)` 或 `![video](/abs/path.mp4)` 渲染
- **不要**展示远程 CDN URL

## 相关页面

- [[media-artifacts]]
- [[gateway]]
- [[integrations]]
- [[../queries/faq]]

## 来源

- `src-tauri/src/gateway/server.rs`（catalog + handle_tool_call）
- `~/.grok-go/agents-guide.md`
- 仓库 `AGENTS.md` 中的 project-doc 段
