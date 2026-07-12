# 概念：模型映射

## 结论

`resolve_model(config, requested)` 决定上游真正使用的模型 id。Codex 可能请求 `gpt-*` 名称，需要映射到 Grok。

## 规则

1. 若 `model_mappings` 有精确键 → 用映射值，reason=`mapped`
2. 否则若请求名像 `grok*` 且不是 image/video/imagine → 原样透传，reason=`passthrough`
3. 否则回落 `default_model`，reason=`default-fallback`
4. 图片/视频路径另有默认：`default_image_model` / `default_video_model`

## 默认映射

- `gpt-5.6` → `grok-4.5`
- `gpt-5.5` → `grok-4.5`

UI Mapping 页可增删映射；`list_model_options` 提供 Codex 侧常见名与 Grok 文本/图/视频候选。

## `/v1/models`

- 优先拉上游 xAI `/models`
- 失败则 curated：默认文本、`grok-4.20-reasoning`、默认图、默认视频

## 相关页面

- [[../modules/config-runtime]]
- [[../modules/gateway]]
- [[../modules/frontend-ui]]

## 来源

- `src-tauri/src/config.rs`（resolve_model）
- `src-tauri/src/gateway/proxy.rs`（list_models / path 默认）
- `src-tauri/src/commands.rs`（list_model_options）
