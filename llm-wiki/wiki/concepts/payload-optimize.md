# 概念：请求体优化与 Files 分流

## 结论

Codex 多轮会把**完整历史**（含 base64 图片、大段 tool 输出）再发一遍；xAI 经本代理不走 OpenAI 式 `previous_response_id` 服务端续写，因此 input token 与 HTTP body 近似线性膨胀。表现是：token 异常飙升、跑一段时间强制停、重启同一会话又停（goal 无法救已污染的上下文）。

`payload_optimize` + `files_api` 在转发前压缩请求体，大文本尽量改成 **Files 上传 + `file_id` 引用**。

## 调研来源

| 来源 | 要点 |
|---|---|
| [xAI Files](https://docs.x.ai/docs/guides/files) | `POST /v1/files` 上传；对话用 `input_file.file_id`；文档走 `attachment_search`，正文不整段塞进 message history |
| [xAI Image Understanding](https://docs.x.ai/docs/guides/image-understanding) | 视觉仍用 `input_image` + URL/data URL；**有图时勿 store 服务端历史**，否则后续请求易失败 |
| CPA (CLIProxyAPI) | 稳定 `prompt_cache_key`；xAI 图/视频生成 **不支持** `file_id`（须 image_url）；input 去重 |
| sub2api | 多账号粘性 / sticky session，保证 file_id 与会话同账号 |

## 网关行为

### 同步（选号前）

1. 检测到图片 → 强制 `store: false`
2. 相同 `data:` 图片去重（后续改为短文本 stub）
3. 全量图片超过预算（默认 8 张）→ 折叠更早的历史图
4. 历史 `function_call_output` / 超长 `input_text` 截断（head+tail）
5. body 超 soft/hard 预算 → 更激进折叠

### 异步（选号后、发上游前）

- 单段文本 ≥ 32KB：上传 xAI Files（账号级 content-hash 缓存）→ 输出改为短说明 + 注入 `input_file.file_id`
- 上传失败则回退截断，不阻断请求

### 代理端点

| 方法 | 路径 |
|---|---|
| POST/GET | `/v1/files` |
| GET/DELETE | `/v1/files/{file_id}` |

客户端也可自行上传后只在 Responses 里带 `file_id`（配合 session sticky）。

## 不能解决的

- 客户端线程本身已塞满巨大 history：网关能砍上行体，但 Codex 本地 transcript 仍大；极端情况建议新开线程
- 图片生成/视频：仍须 base64 或公网 URL，不能只靠 file_id（与 CPA 一致）
- `detail: low` 压缩 vision token 未默认改写（避免误伤设计审图）

## 相关页面

- [[../modules/gateway]]
- [[request-sanitize]]
- [[../playbooks/debug-checklist]]

## 来源

- `src-tauri/src/gateway/payload_optimize.rs`
- `src-tauri/src/gateway/files_api.rs`
- `src-tauri/src/gateway/proxy.rs`
- `src-tauri/src/gateway/server.rs`
