# Query：sub2api / CPA 等反代在缓存、省量、流量分配上做了什么？GrokGo 可借鉴什么？

## 问题

对照 sub2api、CLIProxyAPI（CPA）等订阅反代，它们在 **缓存 / 用量节省 / 流量分配** 上做了哪些工作？哪些值得 GrokGo 学习以提升体验与稳定性？

## 一句话结论

- **真正省订阅额度**的主路径不是「把整段回复缓存在本地再命中」，而是 **上游 Prompt Cache 粘连**（同一会话固定账号 + 稳定 `prompt_cache_key` / 会话 ID）+ **失败冷却与 failover** + **额度/重置时间感知调度**。
- GrokGo 已有：WRR/LRU/最低错误率、请求内 failover、429 本地 cooldown、视频 job sticky、周配额展示（本分支）。
- 最大差距：**会话粘连（session affinity）**、**quota-aware / fill-first / soonest-reset 调度**、**Grok reasoning replay / prompt_cache_key 透传与绑定**、**细粒度错误分类冷却**。

## 对照表

| 能力 | sub2api | CLIProxyAPI (CPA) | GrokGo（本工作树） |
|---|---|---|---|
| 多账号负载均衡 | 多因子打分调度 + sticky | RR / fill-first + session-affinity | WRR / LRU / lowest-error-rate |
| 会话粘连 | session_hash、previous_response_id、sticky wait 队列 | session-affinity + TTL | 仅视频 job / 工具链 sticky token |
| Prompt cache 相关 | 自动派生/注入 `prompt_cache_key`（兼容路径）；sticky 服务 cache | sticky 时 remap cache key；xAI 设 `prompt_cache_key` + `x-grok-conv-id` | 透传为主；sanitize/nuclear 路径会 strip |
| 本地「回复缓存」 | 非主路径；Redis 用于会话/调度状态 | **reasoning replay cache**（thinking 续聊）+ signature cache | 无 |
| 429/配额冷却 | temp-unschedulable + Retry-After；401/403/5xx 分级 | cooldown / reset-quota / quota-exceeded 切换 | 本地 cooldown + 请求内 failover |
| 额度感知选号 | `quota_headroom`、`prefer_soonest_reset`（可配） | 生态工具做 5h/7d 条；issue 有 weekly-reset 选号讨论 | 账号页展示周配额，**未进选号** |
| 并发控制 | 账号/用户 concurrency + 排队 | 连接池/冷却为主 | 无 per-account 并发槽 |
| 缓存计费对齐 | reconcile `cached_tokens` → `cache_read_*` | 用量统计 fork 可见 cache hit | 本地 usage 有 cache token 字段 |

## 源码事实（本地克隆 `/tmp/quota-research`）

### sub2api

1. **Smart Scheduling + sticky**（README 明确特性）  
   - `session_hash` 粘连；Nginx 需 `underscores_in_headers on` 以免丢掉 `session_id`。  
   - OpenAI 调度分层：`previous_response_id` → `session_hash` → load_balance（`openai_account_scheduler.go`）。  
   - 默认权重：priority / load / queue / error_rate / ttft；`session_sticky=3`、`previous_response=5`；`quota_headroom`/`reset` 默认 0（可开）。  
   - **sticky escape**：账号 TTFT EWMA 或错误率恶化时允许逃离粘连（默认 TTFT 15s、error 0.5）。  
   - sticky 排队：`sticky_session_max_waiting` + wait timeout，避免粘连账号忙时直接打散。

2. **use-it-or-lose-it / 额度余量**  
   - `prefer_soonest_reset`：优先用「会话窗口最早重置」的账号。  
   - `quota_headroom`：倾向 7d 剩余更健康的账号（小流量可灰度）。  
   - 与 CPA issue #3066「weekly reset-aware selection」同一产品直觉。

3. **Grok 错误处理**（`openai_gateway_grok.go`）  
   - 401 → 临时下线 10m  
   - 403 → 30m（权益/订阅层，不盲目 refresh loop）  
   - 429 → `Retry-After` 或默认 2m  
   - 5xx → 2m  
   - 响应头解析写入 quota snapshot（与 rate-limit 路径一致，不爬 grok.com 周额度 UI）。

4. **「缓存」在 sub2api 里多指**  
   - Redis 会话/调度状态、API Key 鉴权缓存、计费侧 `cached_tokens` 字段对齐（Claude/Kimi 兼容），**不是**把完整 completion 本地命中当省钱主力。  
   - Codex/兼容路径会 **自动派生 `prompt_cache_key`**（model + system + first_user + tools 等），提高上游 prefix cache 命中。

### CLIProxyAPI（CPA）

1. **routing.strategy**  
   - `round-robin`：均匀消耗，默认。  
   - `fill-first`：先榨干一个健康号再动备份——适合「主号用满再备胎」。  

2. **session-affinity**（默认关）  
   - 从 `metadata.user_id` / `X-Session-ID` / `Session_id` / `conversation_id` / 前几条消息 hash 提取会话。  
   - 绑定 TTL 默认 1h；绑定账号不可用时自动 failover。  
   - 与 fill-first 搭配时，可 remap Codex `prompt_cache_key` / installation identity（`identity-confuse`，偏风控迷信场景）。

3. **xAI 侧缓存相关**（`xai_executor.go` + `internal/cache/xai_reasoning_replay_cache.go`）  
   - 解析/写入 `prompt_cache_key`、`x-grok-conv-id`。  
   - **Reasoning replay cache**：无状态下一轮续聊时回放已加密 reasoning items（TTL 1h、有上限）——这是 **体验/连续性** 缓存，兼间接减少「整段重算失败」。  
   - Claude/Antigravity：**signature cache** 保证 thinking block 签名可续。

4. **冷却与管理面**  
   - `transient-error-cooldown-seconds`、`save-cooldown-status`、`POST /reset-quota` 清运行时 cooldown。  
   - `quota-exceeded.switch-project` 等（多 provider 通用）。  
   - 生态：Quota Inspector / CPA-Manager 做 5h·7d 条、计划权重、不健康账号清理。

### GrokGo 现状（对照）

- 选号：`router.rs` — WRR（默认）、LRU、LowestErrorRate；健康号优先；cooldown 跳过。  
- 稳定性：`proxy.rs` 请求内 failover（有次数上限）；429 本地 cooldown。  
- Sticky：视频 job affinity、image bridge sticky token；**聊天会话无全局 sticky**。  
- 配额：本分支已拉 SuperGrok 周配额展示，**尚未进入 pick_account 打分**。  
- `prompt_cache_key`：一般路径可透传；sanitize/nuclear 路径会移除。

## 值得借鉴的优先级（给 GrokGo）

### P0 — 体验 + 稳定性，投入中等

1. **Session affinity（会话粘连）**  
   - Key 来源优先级：`prompt_cache_key` → `previous_response_id` → 客户端 session/conversation header → 可选 content hash。  
   - 绑定账号 + TTL（1h 级）；账号 cooldown/401 时 failover 并换绑。  
   - **收益**：上游 prompt cache 命中率↑、多轮工具链更稳、账号间「上下文分裂」↓。

2. **把周配额 / rate-limit 喂进选号**  
   - 软因子：`remaining_percent` 低 → 降权（headroom）。  
   - 可选：`resets_at` 将到 → 加权用尽（soonest-reset）。  
   - 与 UI 已有 quota 字段直接复用。

3. **错误分级冷却（对齐 sub2api Grok）**  
   - 401 / 403 / 429(+Retry-After) / 5xx 不同时长与文案；403 不与 token refresh 死循环。

### P1 — 省量 / 调度策略

4. **fill-first 策略**（可选）  
   - 场景：主号日用、备号冷备；与「均摊」WRR 互补，设置里二选一或三选一。

5. **稳定 `prompt_cache_key` 策略**  
   - 粘连账号时 **保留/规范化** key，不要在正常路径误删。  
   - 仅在 failover 换号时改 key 或接受 miss（CPA 在 identity-confuse 场景会 remap）。

6. **xAI reasoning replay（可选，复杂度高）**  
   - 仅当 Codex/多轮 reasoning 在 Grok 上经常断链时再做；本地进程内 LRU + TTL 即可起步。

### P2 — 体验打磨

7. **Per-account 并发上限**（轻量信号量）  
   - 防止单号被本地多客户端打爆 429。  

8. **调度可观测**  
   - 日志/UI：本次选了谁、是否 sticky hit、是否因配额降权、failover 次数（sub2api 有 sticky hit ratio 等 metrics）。

9. **慎做「完整答案语义缓存」**  
   - Agent 工具链/非幂等请求误缓存代价高；除非只对明确幂等 embedding/FAQ 开放。

## 非目标 / 边界

- 拼车计费、用户 API Key 分发、支付：sub2api 核心，**不是** GrokGo 本地桌面网关定位。  
- 爬 grok.com 以外的「黑盒免费额度」：合规与稳定性风险高。  
- 把 480 remaining rate-limit 和 30% SuperGrok credits 混成一个「剩余」：两套计量，UI/调度必须分开。

## 推荐落地顺序（工程）

1. Session affinity + 日志 — **已落地**（`session_affinity.rs` + proxy）  
2. Quota headroom 进 WRR 打分 — **已落地**（`quota_aware_routing`，默认开）  
3. Grok 错误分级 cooldown — **已落地**（401 10m / 403 30m / 429 Retry-After / 5xx×3→2m）  
4. 设置项：fill-first / prefer-soonest-reset — **已落地**（Settings → 模型 → 流量分配）  
5. prompt_cache_key 策略审计 — **已落地**（缺省时注入；sanitize 仍只 strip retention；nuclear 仍可 strip）  
6. （可选）reasoning replay — **未做**（可选 / 高复杂度）  
7. 软并发 — **已落地**（`account_max_concurrency`，默认 6，全忙不阻塞）  

## 来源

- 本地克隆：`/tmp/quota-research/{sub2api,CLIProxyAPI}`（2026-07-12 调研会话）  
- sub2api：`deploy/config.example.yaml`、`service/openai_account_scheduler.go`、`openai_gateway_grok.go`、`openai_compat_prompt_cache_key.go`  
- CPA：`config.example.yaml` routing、`internal/cache/*`、`xai_executor.go`、Management reset-quota  
- GrokGo：`src-tauri/src/router.rs`、`gateway/proxy.rs`  
- 公开：CPA docs load-balancing / fill-first；CPA issue #3066 weekly reset-aware selection  
