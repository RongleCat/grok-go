## <!-- grok-go:agents-guide:start -->

## GrokGo 工具

- 搜索 X / Twitter：直接调用 `x_search`
- 生成图片：调用 `image_gen`（`image_generate` 同义）
- 编辑图片：调用 `image_edit`（原图 URL + 编辑说明）
- 生成视频：调用 `video_generate`
- 编辑视频：调用 `video_edit`（原视频 URL + 编辑说明）

---

<!-- grok-go:agents-guide:end -->

--- project-doc ---

# GrokGo — Agent 指引

## 强制规则

- **参数以 MCP `tools/list` 为准**：直接调用，**禁止** web_search / 翻仓库 / 读 HANDOFF、server.rs 查参数。
- 媒体输入可用：`https://` / `data:` / **本地绝对路径** / `file://`（本地会自动转 data URL）。
- 返回一律是 `~/.grok-go/artifacts/` 下的**绝对本地路径** + `markdown`，用 `![image](/abs/path)` 渲染；**不要**展示远程 CDN URL。

## 工具速查（完整参数）

### `x_search`
- 必填：`query`
- 可选：`allowed_handles` `excluded_handles` `from_date` `to_date`（YYYY-MM-DD）

### `image_gen` / `image_generate`
- 必填：`prompt`
- 可选：`n`(1–4) `model` `size` `quality`(low|medium|high)
- 用途：文生图。立即调用，勿 SVG/Pillow。

### `image_edit`
- 必填：`prompt` + `image_url`（URL 或本地路径）
- 可选：`model`

### `video_generate`（文生视频 / 图生视频 / 多图参考）
- 必填：`prompt`
- 模式（三选一）：
  1. 文生视频：仅 `prompt`
  2. 图生视频：`prompt` + `image_url`（首帧）
  3. 多图参考：`prompt` + `reference_image_urls`（1–7，勿与 `image_url` 同用）
- 可选：`duration`(1–15) `aspect_ratio`(1:1|16:9|9:16|4:3|3:4|3:2|2:3) `resolution`(480p|720p|1080p) `model`
- 示例图生视频：
  `{"prompt":"轻推镜头，微风吹动毛发","image_url":"/abs/path.png","duration":6}`

### `video_edit`
- 必填：`prompt` + `video_url`（URL 或本地路径）
- 可选：`model`
- 注意：编辑不支持自定义 duration/aspect_ratio

## HTTP 回退（MCP 不可用时）

```bash
# localToken 来自 ~/.grok-go/config.json
curl -s http://127.0.0.1:8787/v1/images/generations \
  -H "Authorization: Bearer <localToken>" \
  -H "Content-Type: application/json" \
  -d '{"model":"grok-imagine-image-quality","prompt":"<描述>","n":1}'
```

## 健康检查

```bash
curl -s http://127.0.0.1:8787/health
# 期望：running=true
```

## 其它

- Responses API Base：`http://127.0.0.1:8787/v1`
- MCP：`http://127.0.0.1:8787/mcp`（需 Bearer localToken）
- 产物目录：`~/.grok-go/artifacts/`
