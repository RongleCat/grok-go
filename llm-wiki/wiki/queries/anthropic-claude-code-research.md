# 调研：Anthropic Messages 兼容 → Claude Code

## 结论

要让 **Claude Code** 使用 GrokGo 的 xAI 账号池，网关需新增 **Anthropic Messages API 兼容层**，而不是让用户改用 OpenAI base URL。Claude Code 固定走：

```text
POST {ANTHROPIC_BASE_URL}/v1/messages
```

推荐实现路径：

```text
Claude Code  --Messages-->  GrokGo /v1/messages
                              │
                              ├─ anthropic → OpenAI chat/completions
                              ├─ 现有账号 failover / 选号 / usage
                              └─ OpenAI 响应/SSE → Anthropic message / SSE
```

**工具调用**是生死线：必须双向保真 `tool_use.id` ↔ `tool_calls[].id`，流式输出完整的  
`content_block_start(tool_use)` + `input_json_delta` + `stop_reason=tool_use`。

## 为什么不做 Messages 直转 Responses

| 方案 | 优点 | 缺点 |
|---|---|---|
| Messages ⇄ Chat Completions | 与 CC Switch/DeepSeek 同类中转一致；tool_calls 映射成熟 | 少用 GrokGo 已有的 Responses 专用 sanitize |
| Messages ⇄ Responses | 可复用 empty-completion、image tool loop | input item 模型差异大；工具/多轮回归成本高 |

**Phase 1 选 Chat Completions。** Responses 作为后续增强。

## 必须实现的端点

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/v1/messages` | 主路径（stream + 非 stream） |
| POST | `/v1/messages/count_tokens` | Claude Code 会调；可做粗略估算 |
| GET | `/v1/models` | 已有；鉴权需兼容 `x-api-key` |

云端专用（sessions/agents/…）**不实现**，返回明确 404/501 即可。

## 鉴权

同时接受：

1. `Authorization: Bearer <localToken>`
2. `x-api-key: <localToken>`

错误体尽量贴近 Anthropic：`{"type":"error","error":{"type":"authentication_error","message":"…"}}`（至少在 messages 路径）。

## 工具调用映射（不可省略）

### 请求

- `tools[].input_schema` → `tools[].function.parameters`
- `tool_choice` **对象映射**（禁止原样透传）
- assistant 历史中的 `tool_use` → `tool_calls`（`arguments` 为 JSON **字符串**）
- user 的 `tool_result` → `role: tool` + `tool_call_id`
- 剥离：`thinking` 块、`cache_control`、未知 beta 字段

### 响应 / SSE

- `tool_calls` → `content[].type=tool_use`（`input` 为对象）
- `finish_reason=tool_calls` → `stop_reason=tool_use`
- 流式：按 index 维护多个 tool block；`partial_json` 追加 arguments 分片
- `content_block_start` 的 tool_use **必须含 id、name**（`input` 建议 `{}`）

## 模型映射

Claude Code 会发 `claude-sonnet-…` / `claude-haiku-…` / `claude-opus-…`，或用户已映射的 `grok-*`。

策略：

1. 若已是 `grok-*` / 配置表内模型 → `resolve_model` 原逻辑
2. 否则按名字含 haiku/opus/sonnet 映射到可配置默认（先用 `default_model`，后续可加 `anthropic_haiku_model` 等配置）

## 与现有模块边界

| 模块 | 改动 |
|---|---|
| `gateway/anthropic.rs`（新） | 请求/响应/SSE 转换 + 单测 |
| `gateway/server.rs` | 注册 `/v1/messages*` |
| `gateway/proxy.rs` | 鉴权认 `x-api-key`；messages 专用 handler 调转换后走 chat 上游 |
| `integrations` / UI（后续） | 一键写 Claude Code settings / CC Switch claude provider |
| `llm-wiki` | 本页 + gateway 模块页更新 |

## 验收标准（工具不翻车）

1. 非流式：单轮 text  
2. 非流式：单 tool_use → 客户端回 tool_result → 第二轮 text  
3. 流式：同上两轮  
4. 并行双 tool_use（若模型产出）id 不串  
5. `tool_choice: any` / 指定 tool 名不 4xx  
6. Claude Code 真实会话：Read/Bash 等内置工具可跑通至少一轮  

## 开源方案对比（2026-07-14）

目标：找**可直接复用**或**可抄细节**的 Messages ⇄ Chat Completions 实现，避免自研踩工具调用/SSE 坑。

### 推荐优先级（给 GrokGo）

| 优先级 | 项目 | 形态 | License | 为何相关 |
|---|---|---|---|---|
| **A 首选库** | [TokenFleet-AI/llm-bridge-rust](https://github.com/TokenFleet-AI/llm-bridge-rust) (`llm-bridge-core` crates.io **0.5.0**) | **纯协议库**（无鉴权/路由） | **Apache-2.0** | Anthropic↔Chat↔Responses 双向 + SSE；fixture + 大量单测；`tool_choice` 映射正确；明确忽略 Claude Code 多余字段 |
| **A 对照实现** | [m0n0x41d/anthropic-proxy-rs](https://github.com/m0n0x41d/anthropic-proxy-rs) (crates.io `anthropic-proxy`) | 独立 Axum 代理二进制 | **MIT** | 专为 Claude Code 设计；schema normalize、BatchTool 过滤、流式 tool_use 状态机完整；**但 `tool_choice` 被硬编码丢弃为 `None`** |
| **B 本地已有** | 本机 `cc-switch` `proxy/providers/transform.rs` + `streaming.rs` | 产品内嵌转换 | 产品自有 | 已验证 OpenRouter 路径；`tool_choice` 原样透传有缺陷；流式逻辑可抄 |
| **B 巨型网关** | [router-for-me/CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) ★41k | Go 多协议网关 | **MIT** | `internal/translator/openai/claude/*` 含 request/response 测试；支持 Claude/Codex/Grok Build 等多前端，**细节金矿**，不宜整仓嵌入 |
| **B 路由层** | [musistudio/claude-code-router](https://github.com/musistudio/claude-code-router) ★35k | TS 路由/插件 | **MIT** | Claude Code 生态事实标准之一；偏路由/多 provider，不是薄转换库 |
| **C 参考勿嵌** | [QuantumNous/new-api](https://github.com/QuantumNous/new-api) ★42k | Go 分发系统 | **AGPL-3.0** | `relay/claude_handler.go` 等交叉转换成熟；**AGPL 不适合并进 GrokGo** |
| **C 旁路** | [BerriAI/litellm](https://github.com/BerriAI/litellm) ★53k | Python 网关 | 特殊/Other | 转换能力最全，但 Python 重、license 不清晰，只适合对拍行为 |
| **C 小库** | [codeproxy-ai/core](https://github.com/codeproxy-ai/core) (`@codeproxy/core`) | TS 零依赖 | **MIT** | 方向是 **→ Responses**（给 Codex 用），与我们 Messages←Chat 主路径相反，可参考 stream 事件 |

### 不推荐整仓依赖 / 侧挂的原因

1. **CLIProxyAPI / CCR / new-api / LiteLLM** 都是「完整产品」，和 GrokGo 的 OAuth 池、选号、MCP、usage DB 重叠；侧挂等于双网关运维。
2. **anthropic-proxy-rs** 是二进制不是 lib；且 `translate_request` 里 `tool_choice: None`——Claude Code 强制 tool 场景会丢语义。
3. **llm-bridge-core** 是目前最贴「只做协议、可嵌入」的 Rust 选择；注意：
   - 声明 **MSRV / toolchain 偏新**（仓库 `rust-toolchain.toml` 钉 **1.96**，edition 2024）；本机若用 1.92 需先验证能否编过 0.5.0。
   - stars 很少、维护主体新；应 **依赖 + 自备回归测试**，不要盲信。
   - 未看到 `disable_parallel_tool_use` → `parallel_tool_calls: false` 的映射（与 anthropic-proxy / cc-switch 一样缺），GrokGo 需自补。

### 各方案能补的「细节清单」

从源码对拍，成熟实现已经处理好、自研时务必覆盖：

| 细节 | llm-bridge | anthropic-proxy-rs | cc-switch | 说明 |
|---|---|---|---|---|
| system string / text[] 合并 | ✅ | ✅ | ✅ | 多 system block join |
| tool_use → tool_calls（arguments **字符串**） | ✅ | ✅ | ✅ | |
| tool_result → role=tool | ✅ | ✅（数组 content 展平） | ✅ | 数组 content 必须展平 |
| tool_choice auto/any/none/tool | ✅ 正确 | ❌ 丢弃 | ⚠️ 原样透传 | **优先抄 llm-bridge** |
| 过滤 BatchTool | ? | ✅ | ✅ | Claude Code 偶发 |
| schema 去 `format:uri` / null / object.required | 弱 | ✅ 强 normalize | 弱 | **抄 anthropic-proxy normalize_schema** |
| 流式 message_start → block → message_delta → stop | ✅ | ✅ + 单测 | ✅ | |
| input_json_delta 分片 | ✅ | ✅ | ✅ | |
| thinking ↔ reasoning | ✅ | ✅ | 跳过 thinking | xAI 可按需 |
| stop_reason 映射表 | ✅ 集中表 | ✅ | ✅ | tool_calls→tool_use |
| 忽略 cache_control / 未知字段 | ✅ 注释写明 | 模型字段可选 | 部分 | Claude Code 会塞 beta 字段 |
| Anthropic→Responses | ✅ | ❌ | ❌ | 若未来走 xAI Responses 可复用 |
| disable_parallel_tool_use | ❌ | ❌ | ❌ | **三家都缺，GrokGo 自补** |

### 给 GrokGo 的落地建议

**方案 1（推荐）：依赖 `llm-bridge-core` + 薄适配**

```text
POST /v1/messages
  → llm_bridge_core::transform::anthropic_to_openai
  → 现有 proxy chat/completions 上游（选号/failover/usage）
  → 非流：response 反变换
  → 流：stream::transform_stream_to_anthropic_sse + StreamState
```

自补层：

- 鉴权：`x-api-key` + Bearer = `localToken`
- 模型：claude-* → grok-* 映射
- `disable_parallel_tool_use`
- schema normalize（从 anthropic-proxy 抄 `normalize_schema`）
- `count_tokens` 粗估
- 错误体 Anthropic 形状

**方案 2：不引依赖，移植细节**

从以下文件 **抄逻辑不抄整仓**（MIT/Apache 兼容，注明出处）：

- `llm-bridge-rust`：`stop_reason.rs`、`anthropic_to_openai.rs` tool_choice、stream 状态机
- `anthropic-proxy-rs`：`normalize_schema`、`is_batch_tool`、stream `emit_tool_calls`
- `CLIProxyAPI`：`internal/translator/openai/claude/*_test.go` 作行为对拍用例
- 本机 `cc-switch` streaming 作第二参照

**方案 3：用户侧组合（产品零开发）**

```text
Claude Code → anthropic-proxy-rs / CCR → GrokGo /v1/chat/completions
```

可行但体验差（双进程、tool_choice 坑、集成页无法一键），**不作为产品主路径**。

### 结论（开源选型）

| 问题 | 答案 |
|---|---|
| 有没有成熟开源可直接用？ | **有库级方案：`llm-bridge-core`（Apache-2.0）**；有成熟代理：`anthropic-proxy-rs` / CLIProxyAPI / CCR，但不适合整仓嵌入 |
| 能否弥补细节？ | **能**：tool_choice、schema normalize、流式 tool 状态机、BatchTool、system 多块——应对照上表逐项补 |
| 是否还要自研？ | 网关路由/鉴权/选号必须自研；**协议转换优先复用库或移植**，不要从零写 SSE 状态机 |

### 落地决定（2026-07-14）

**自包含移植**（方案 2），不依赖 `llm-bridge-core`：

- 原因：GrokGo 需长期自维护；避免 edition 2024 / 新 toolchain 绑定；可把 xAI 选号、payload 优化、鉴权与协议层合在一起。
- 实现：`src-tauri/src/gateway/anthropic/` + `proxy::proxy_anthropic_messages`
- 用户配置：

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8787",
    "ANTHROPIC_AUTH_TOKEN": "<localToken>",
    "ANTHROPIC_MODEL": "grok-4.5",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "grok-4.3",
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "grok-4.5",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "grok-4.5"
  }
}
```

注意：`ANTHROPIC_BASE_URL` **不要**带 `/v1`。

## 参考

- raw：[[../../raw/anthropic-claude-code-compat]]
- 对照实现：本机 `cc-switch` `transform.rs` / `streaming.rs`
- 协议类型：`@anthropic-ai/sdk` Message / MessageStreamEvent
- 本地浅克隆对拍：`/tmp/llm-bridge-rust`、`/tmp/anthropic-proxy-rs`（调研用，不进仓库）

## 来源

- Claude Code 2.1.185 二进制路径字符串
- 本机 `~/.claude/settings.json`（DeepSeek Anthropic 兼容已在用）
- GrokGo `gateway/*`、`integrations.rs`
- crates.io：`llm-bridge-core`、`anthropic-proxy`
- GitHub：llm-bridge-rust、anthropic-proxy-rs、CLIProxyAPI、claude-code-router、new-api、codeproxy-ai/core
