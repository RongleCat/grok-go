# Raw notes: Anthropic Messages + Claude Code 接入 GrokGo

收集时间：2026-07-14  
目的：让 Claude Code 通过 `ANTHROPIC_BASE_URL` 指向 GrokGo，走 xAI OAuth 池。

## Claude Code 客户端约定（本机实测）

环境变量（`~/.claude/settings.json` 的 `env`）：

| 变量 | 作用 |
|---|---|
| `ANTHROPIC_BASE_URL` | 网关根，**不含** `/v1` 后缀时客户端会拼 `/v1/messages`；DeepSeek 类中转常写成 `https://host/anthropic` |
| `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_API_KEY` | 认证；CC 会发 `Authorization: Bearer …` 和/或 `x-api-key` |
| `ANTHROPIC_MODEL` | 默认模型 id（可直接写 `grok-4.5`） |
| `ANTHROPIC_DEFAULT_{HAIKU,SONNET,OPUS}_MODEL` | 按 Claude 档位名映射到真实上游模型 |

完整请求路径：

```text
POST {ANTHROPIC_BASE_URL}/v1/messages
```

从 Claude Code 2.1.185 二进制字符串提取的相关路径频次（节选）：

- `/v1/messages`（主对话）
- `/v1/messages/count_tokens`
- `/v1/models`
- 另有 sessions/agents 等官方云端路径——**自定义 base 时通常不要求本地实现**

Headers：

- `anthropic-version: 2023-06-01`（常见）
- `x-api-key` 或 `Authorization: Bearer`
- `content-type: application/json`
- 可选 `anthropic-beta: …`

## Anthropic Messages 协议要点（SDK 类型 + 社区实现）

### 请求

```json
{
  "model": "claude-…",
  "max_tokens": 1024,
  "system": "…" | [{"type":"text","text":"…","cache_control":…}],
  "messages": [
    {"role":"user"|"assistant", "content": "…" | [ContentBlock]}
  ],
  "tools": [{"name","description","input_schema"}],
  "tool_choice": {"type":"auto"|"any"|"none"|"tool","name"?,"disable_parallel_tool_use"?},
  "stream": true,
  "temperature": 0.7,
  "top_p": …,
  "stop_sequences": […],
  "thinking": {"type":"enabled"|"disabled", …}
}
```

Content blocks：

| type | 方向 | 含义 |
|---|---|---|
| `text` | 双向 | 文本 |
| `image` | 入 | `{source:{type:base64,media_type,data}}` |
| `tool_use` | 出 / 历史 assistant | `{id,name,input}` |
| `tool_result` | 入 user | `{tool_use_id, content, is_error?}` |
| `thinking` | 双向 | 推理块；上游无对应字段时应剥离 |

### 非流式响应

```json
{
  "id": "msg_…",
  "type": "message",
  "role": "assistant",
  "content": [
    {"type":"text","text":"…"},
    {"type":"tool_use","id":"…","name":"…","input":{}}
  ],
  "model": "…",
  "stop_reason": "end_turn"|"tool_use"|"max_tokens"|"stop_sequence"|"pause_turn"|"refusal",
  "stop_sequence": null,
  "usage": {"input_tokens":N,"output_tokens":M}
}
```

### 流式 SSE（event 名 + data JSON）

顺序契约（Claude Code / Anthropic SDK 强依赖）：

1. `message_start` → 含 `message` 骨架（id/role/model/usage 初值）
2. 对每个 content block：
   - `content_block_start`（tool_use 时 **必须** 带 `id`+`name`，建议 `input:{}`）
   - 0+ `content_block_delta`
     - text → `{"type":"text_delta","text":"…"}`
     - tool_use → `{"type":"input_json_delta","partial_json":"…"}`（参数字符串分片）
   - `content_block_stop`
3. `message_delta` → `delta.stop_reason` + `usage.output_tokens`
4. `message_stop`

OpenAI Chat Completions SSE → Anthropic 映射：

| OpenAI | Anthropic |
|---|---|
| first chunk | `message_start` |
| `delta.content` | text block start/delta |
| `delta.tool_calls[].id/name` | `content_block_start` tool_use |
| `delta.tool_calls[].function.arguments` | `input_json_delta.partial_json` |
| `finish_reason` | `message_delta.stop_reason` + block stop |
| `data: [DONE]` | `message_stop` |

`finish_reason` → `stop_reason`：

| OpenAI | Anthropic |
|---|---|
| `stop` | `end_turn` |
| `length` | `max_tokens` |
| `tool_calls` | `tool_use` |
| 其他 | `end_turn`（保守） |

### tool_choice 映射（**不能**原样透传给 OpenAI）

| Anthropic | OpenAI |
|---|---|
| `{type:auto}` | `"auto"` |
| `{type:any}` | `"required"` |
| `{type:none}` | `"none"` |
| `{type:tool,name}` | `{type:function,function:{name}}` |
| `disable_parallel_tool_use:true` | `parallel_tool_calls: false` |

### tools 映射

```text
Anthropic: { name, description, input_schema }
OpenAI:    { type: "function", function: { name, description, parameters: input_schema } }
```

schema 清理：去掉部分上游不认的 `format: "uri"` 等。

### 消息映射（工具闭环）

1. `system` → 首条 `role:system`（多块 text 拼接）
2. assistant `tool_use` 块 → 同条 assistant 的 `tool_calls[]`（arguments 为 **JSON 字符串**）
3. user `tool_result` 块 → **独立** `role:tool` 消息（`tool_call_id` = `tool_use_id`）
4. 同一 user 消息里 text + tool_result 并存时：先/后拆成多条 OpenAI 消息
5. **`tool_use.id` 必须原样回传**，否则 Claude Code 无法匹配 tool_result

## 上游选择：Chat Completions vs Responses

GrokGo 现状：

- Codex / Grok Build：`/v1/responses`（已有 sanitize / empty-completion / image tool loop）
- OpenAI 兼容：`/v1/chat/completions` → 直转 xAI

Claude Code 原生是 **Messages**，业界中转（CC Switch OpenRouter 路径、DeepSeek Anthropic 兼容层）普遍：

```text
Messages ⇄ Chat Completions
```

选择 Chat Completions 的原因：

1. tool_calls 字段与 Anthropic tool_use 一一对应，实现成熟（cc-switch 已验证）
2. xAI 公开支持 chat completions + tools
3. 避免再把 Anthropic 内容块硬塞进 Responses `input` 数组（成本更高、回归面更大）

后续若 xAI Chat 对某类 tool schema 不稳，可再评估 Messages → Responses。

## 鉴权

GrokGo 当前只认 `Authorization: Bearer <localToken>`。  
Claude Code / Anthropic SDK 还发 `x-api-key`。  
兼容层必须同时接受：

- `Authorization: Bearer <localToken>`
- `x-api-key: <localToken>`

## 与 CC Switch 的关系

- GrokGo 已能把 **Codex** provider 写入 CC Switch（`wire_api=responses`）
- **Claude** app_type 尚未导出；用户也可手动设 `ANTHROPIC_BASE_URL=http://127.0.0.1:PORT`
- 可参考实现：`cc-switch/src-tauri/src/proxy/providers/transform.rs` + `streaming.rs`（注意其 `tool_choice` 原样透传有缺陷，GrokGo 需正确映射）

## 风险清单（工具调用）

1. 流式 tool_use 缺 `id` → Claude Code 工具循环断裂  
2. `arguments` 非合法 JSON 分片拼接失败 → input 解析错误  
3. 多 tool 并行时 index 错乱 → 块错位  
4. `stop_reason` 仍为 `end_turn` 但 content 含 tool_use → 客户端可能不执行工具  
5. `tool_result.content` 为数组时未展平 → 上游 4xx  
6. Anthropic `tool_choice` 对象透传 → xAI 拒识  
7. 未剥离 `cache_control` / `thinking` / beta 字段 → 上游 4xx  
8. 鉴权只认 Bearer → Claude Code 401  

## 建议接入配置（用户侧）

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8787",
    "ANTHROPIC_AUTH_TOKEN": "<GrokGo localToken>",
    "ANTHROPIC_MODEL": "grok-4.5",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "grok-4.3",
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "grok-4.5",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "grok-4.5"
  }
}
```

说明：`ANTHROPIC_BASE_URL` **不要**带 `/v1`（与 DeepSeek `…/anthropic` 同类：客户端再拼 `/v1/messages`）。

## 参考源

- 本机 Claude Code 2.1.185 路径字符串
- `@anthropic-ai/sdk` Message / Stream 类型（OneKeyClaw 依赖树）
- `cc-switch` transform/streaming
- GrokGo `gateway/proxy.rs`、`server.rs`
