# 概念：Agent premature stop 恢复

## 结论

Grok 有时会过早 `status=completed`，Codex 因此中途停任务。GrokGo 用**任务无关的结构规则**在网关侧恢复 agent loop——不解析业务（报销/车票等）。

## 触发条件（结构判定）

路径 `/v1/responses` 且 `emptyCompletionRetry`（默认 true），且：

```
status=completed ∧ 无 error/incomplete
∧ (
    无 tool call 且无可见 message（reasoning-only / 空 output）
    ∨ (request.tools 非空 ∧ 无 tool call ∧ message 不是明确终态)
  )
```

**明确终态**（不恢复）：

- message 长度 ≥ 280 字
- 向用户提问（含 `?` / `？`）
- 通用交付特征：`saved to` / `written to` / 绝对路径 /「已完成」等

不再使用「先/正在/let me」等意图词表。

## 行为

1. **非流式 JSON**：判定空完成 → 静默重试 1 次  
2. **流式（默认）**：**真流式透传**（与 Grok Build 一致），**不**整段缓冲——否则 TTFT = 整段生成时间，体感极慢  
3. **流式 + `emptyCompletionStreamBuffer=true`**：缓冲 SSE（上限 24MB）→ 判定 → 恢复 → SSE 回放（牺牲首字速度换 agent 续跑）  
4. **软重试**：`stream=false` + 钉死 shell 类 `tool_choice` + recovery nudge  
5. **硬兜底**：注入合成 `function_call`（`echo grok-go-continue`）

## 配置

| 字段 | 默认 | 含义 |
|---|---|---|
| `emptyCompletionRetry` | `true` | 非流式空完成恢复；流式还需下面开关 |
| `emptyCompletionStreamBuffer` | **`false`** | 为 true 才缓冲整段 SSE 做流式恢复（会拖慢首字） |

## 平面策略（console / experimental / native Build）

| 客户端 | 上游 | empty-completion 恢复 | 说明 |
|---|---|---|---|
| Codex/OpenAI 走 console | `api.x.ai` | **开** | 原有逻辑 |
| Codex/OpenAI + `experimentalImpersonateGrokBuild` | cli-chat-proxy | **开** | 仿冒 Build 线，但仍是 Codex agent loop |
| 原生 Grok Build TUI | cli-chat-proxy | **关** | 官方 agent loop 自管 tool turn |

另：有 `tools` 的 agent 回合在恢复开启时会 **强制上游 `stream=false`**，再在客户端要 SSE 时用 `responses_json_to_sse` 回放——否则 Codex 会在 `response.completed`（仅 reasoning）处结束任务（见会话 `019f6852-8442-7fc2-89a3-360cdca9f9b6`）。

## 相关页面

- [[../modules/gateway]]
- [[../modules/config-runtime]]
- [[../playbooks/debug-checklist]]

## 来源

- `src-tauri/src/gateway/empty_completion.rs`
- `src-tauri/src/gateway/proxy.rs`
- `src-tauri/src/config.rs`
