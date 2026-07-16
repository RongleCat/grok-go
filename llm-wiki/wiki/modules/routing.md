# 模块：账号路由与 Failover

## 结论

`router.rs` 负责从已登录账号里挑一个；`proxy.rs` 的 `send_with_account_failover` 在同一次请求内对 401/403/429/5xx/传输错误尝试换号（最多 3 次或可用账号数）。

## 选号策略（`routing_strategy`）

| 策略 | 行为 |
|---|---|
| `weighted-round-robin`（默认） | 按 `weight` 加权轮询 |
| `least-recently-used` | 最久未成功的优先 |
| `lowest-error-rate` | `consecutive_failures` 最低者（并列随机） |

## 候选过滤

1. `enabled`
2. 有 access 或 refresh token
3. **媒体能力**（`MediaCapability`）：图片路径要求 `supports_image`；视频路径要求 `supports_video`；文本为 `Any`
4. 不在 exclude 列表（本请求已失败的号）
5. 非 Disabled；Cooldown 仅当 cooldown 已过期
6. 优先 `Healthy && consecutive_failures==0`；否则按失败次数软排序

入口：`pick_account_decision_cap`；`send_with_account_failover` / MCP `call_upstream` / image_bridge 按 upstream path 自动推导能力。

## Failover 细节

- 触发状态：401、403、429、5xx，或 transport 错误、refresh 失败
- 401 会先 **同账号 refresh 再重发一次**，仍失败才换号
- 429 会读 `Retry-After` 等头，进入本地 cooldown
- 成功路径尽量只 patch 内存 cache，减少每次写 `auth.json`
- **换号 / sticky 不匹配时**：剥离 body 里的 `previous_response_id`（账号态 response 链），保留 `prompt_cache_key`（客户端稳定标签，可在新号上重暖 cache）。避免把 A 的 id 打到 B（console 与 **仿冒 cli-chat-proxy** 均适用）

## 错误信息语义

- 无登录账号：提示去 Accounts 完成 OAuth
- 全部 cooldown：返回最早解禁时间
- 本请求已无剩余号：`no more accounts available for failover`

## Grok Build 原生平面

- 客户端带 `X-XAI-Token-Auth: xai-grok-cli` / `x-grok-model-override` 时走 **cli-chat-proxy** 上游（`cli_chat_proxy_base_url`），不是 console `api.x.ai`。
- 会话黏连与 `prompt_cache_key` / `x-grok-conv-id` 与 Codex 路径共用；并透传 Grok Build 所需 CLI 头。
- **缓存 / token 护栏**（防止多轮 miss 与重复计费）：
  - sticky key 优先读 `x-grok-conv-id` / `x-grok-session-id` / `x-grok-agent-id`
  - 上游 `x-grok-conv-id` **优先透传客户端原值**（勿用 seed 覆盖）
  - Build 平面保留 `previous_response_id` / `prompt_cache_retention`；**不**跑 empty-completion 静默重试、nuclear strip、Files offload
- **必须透传** `GET /v1/user?include=subscription` 与 **`GET /v1/settings`**：
  - `/user`：订阅枚举（GrokPro / XPremiumPlus…）
  - `/settings`：远程配置含 **`allow_access`**（真正开关；缺路由会一直 subscription required）
  - 不要把 `subscriptionTiers` 改写成非 API 枚举（如 `SuperGrok` 字符串会触发 `paywall_check_no_subscription`）
- 集成注入见 [[integrations]]。

## 相关页面

- [[auth-oauth]]
- [[gateway]]
- [[config-runtime]]
- [[integrations]]

## 来源

- `src-tauri/src/router.rs`
- `src-tauri/src/gateway/proxy.rs`
