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

1. **非流式 JSON / agent tools 强制非流式**：判定 premature → **多阶段恢复**  
2. **流式（默认）**：**真流式透传**；有 tools 的 agent 回合会强制上游 `stream=false` 以便恢复后再 SSE 回放  
3. **流式 + `emptyCompletionStreamBuffer=true`**：缓冲 SSE（上限 24MB）→ 判定 → 恢复 → SSE 回放  

### 多阶段恢复（对齐 Grok Build 采样层 + Codex 硬兜底）

| 阶段 | 次数 | 行为 |
|---|---|---|
| A 透明重采 | ≤2 | 原请求体 + `stream=false`，**保留** `previous_response_id` / cache key（Build 风） |
| B 软恢复 | ≤1 | 钉 shell `tool_choice` + recovery nudge；build 面保留 `prompt_cache_key` |
| C 硬兜底 | 1 | 合成中性 `function_call`（`echo grok-go-continue`），保证 Codex 不收工 |

分类日志：`ReasoningOnly` / `NoToolNonFinal`（对应 Build `EmptyReason`）。

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
