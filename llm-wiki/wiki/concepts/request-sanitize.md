# 概念：Responses 请求清洗

## 结论

Codex 发出的 Responses payload 含 xAI 不支持的字段与 tool 类型。`sanitize.rs` 在上行转换、下行回写，保证 Codex 多工具会话能在 Grok 上跑。

## xAI 接受的 tool types

`function | web_search | x_search | collections_search | file_search | code_execution | code_interpreter | mcp | shell`

## 关键转换

| 方向 | 行为 |
|---|---|
| 请求 | `custom` tool → `function`；记录名称以便回写 |
| 请求 | 去掉 `previous_response_id`、`context_management` 等不兼容键 |
| 请求 | 规范 content parts / function_call 形态 |
| 请求 | 识别 image_gen 类工具，供代理服务端履行 |
| 响应 / SSE | 匹配的 `function_call` 改回 `custom_tool_call` |
| 错误启发式 | compaction blob / model input 错误检测辅助 |

## 为何重要

没有这层，Codex 的 `apply_patch` 等 custom tools 会在上游直接失败。改 sanitize 属于高风险横切逻辑，必须补单测（文件内已有若干 `#[cfg(test)]`）。

## 相关页面

- [[../modules/gateway]]
- [[../modules/mcp-tools]]
- [[../playbooks/debug-checklist]]

## 来源

- `src-tauri/src/gateway/sanitize.rs`
- `src-tauri/src/gateway/proxy.rs`
