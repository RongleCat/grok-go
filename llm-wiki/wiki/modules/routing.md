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
3. 不在 exclude 列表（本请求已失败的号）
4. 非 Disabled；Cooldown 仅当 cooldown 已过期
5. 优先 `Healthy && consecutive_failures==0`；否则按失败次数软排序

## Failover 细节

- 触发状态：401、403、429、5xx，或 transport 错误、refresh 失败
- 401 会先 **同账号 refresh 再重发一次**，仍失败才换号
- 429 会读 `Retry-After` 等头，进入本地 cooldown
- 成功路径尽量只 patch 内存 cache，减少每次写 `auth.json`

## 错误信息语义

- 无登录账号：提示去 Accounts 完成 OAuth
- 全部 cooldown：返回最早解禁时间
- 本请求已无剩余号：`no more accounts available for failover`

## 相关页面

- [[auth-oauth]]
- [[gateway]]
- [[config-runtime]]

## 来源

- `src-tauri/src/router.rs`
- `src-tauri/src/gateway/proxy.rs`
