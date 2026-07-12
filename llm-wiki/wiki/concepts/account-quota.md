# 概念：账号剩余用量（SuperGrok Credits / Rate Limit）

## 结论

xAI 有 **两套几乎独立的用量体系**，UI 截图里的 “Weekly SuperGrok Limit 70% used / Resets July 18…” 属于 **订阅 Credits 配额**，不是 `api.x.ai` 响应头里的 RPM/TPM rate limit。

| 维度 | SuperGrok 周配额（截图） | API Rate Limit 头 |
|---|---|---|
| 用户可见 | Weekly SuperGrok Limit、API / Grok Build 分产品占比、重置时间 | limit / remaining |
| 数据源 | `POST https://grok.com/grok_api_v2.GrokBuildBilling/GetGrokCreditsConfig` | 上游 `api.x.ai` 响应头 `x-ratelimit-*` |
| 鉴权 | Grok Build / grok-cli OAuth access token（Bearer）即可；cookie 可选 | 同一 OAuth / API key |
| 是否主动可查 | **是**（空 protobuf 请求） | 被动观察；可用 1 token probe 主动打一次 |
| 当前 GrokGo | **未接** | 已接 `apply_rate_limit_headers`，且成功响应常 **没有 reset** |

实测（2026-07-12，本机 OAuth 账号）：

- 账号 A：`usage_percent=72`，product `1=50`（API）、`2=22`（Grok Build），`reset=2026-07-17T19:33:08Z` = 上海时间 **2026-07-18 03:33**（与截图一致）。
- 账号 B：`usage_percent=1`，product `1=1`，周窗口起点/终点约登录后 7 天。
- `POST https://api.x.ai/v1/responses` 成功响应头示例：
  - `x-ratelimit-limit-requests: 480`
  - `x-ratelimit-remaining-requests: 480`
  - `x-ratelimit-limit-tokens: 10000000`
  - `x-ratelimit-remaining-tokens: 10000000`
  - **无** `x-ratelimit-reset-*`、**无** `x-subscription-tier` / `x-entitlement-status`。

因此：要把账号「剩余用量」做成截图那种进度条，**必须走 GrokBuildBilling gRPC-web**；只盯 rate-limit 头只能做 RPM/TPM 调度，不能还原 Weekly SuperGrok Limit。

## 细节

### 1. Grok Build / CodexBar 路线（推荐主路径）

官方 Grok Build CLI 的 `/usage` 最终对应 billing；CodexBar 文档与实现是目前最完整的公开逆向：

1. **首选（理想）**：`grok agent stdio` JSON-RPC 方法 `x.ai/billing`
   - 返回 JSON：`billingCycle`、`monthlyLimit`、`usage.totalUsed` 等。
   - 已知限制：部分 CLI 版本在 agent-stdio 上返回 `Method not found`，只在交互 TUI 可用。
2. **可靠 fallback**：gRPC-web
   - URL：`https://grok.com/grok_api_v2.GrokBuildBilling/GetGrokCreditsConfig`
   - Method：`POST`
   - Body：5 字节 empty frame `00 00 00 00 00`（flags=0 + length=0）
   - Headers：
     - `Authorization: Bearer <access_token>`
     - `Content-Type: application/grpc-web+proto`
     - `x-grpc-web: 1`
     - `x-user-agent: connect-es/2.1.1`
     - `Origin: https://grok.com`
     - `Referer: https://grok.com/?_s=usage`
   - Cookie：CodexBar 会优先 cookie+bearer，再 cookie-only；**本机实测仅 Bearer 即可**（与 GrokGo 现有 OAuth 权限面一致：`referrer=grok-build` + `grok-cli:access api:access conversations:read/write`）。

#### 响应 protobuf（字段路径为启发式，非官方 .proto）

外层 gRPC-web data frame → 顶层 field `1` message：

| 路径 | 类型 | 含义（实测） |
|---|---|---|
| `1.1` | fixed32 float | 总已用百分比（截图 `70% used`） |
| `1.4` | google.protobuf.Timestamp 风格 | 当前周期开始（seconds + nanos） |
| `1.5` | Timestamp | **重置时间**（截图 Resets …） |
| `1.7` repeated | `{ id: varint, percent?: fixed32 }` | 产品拆分：`1=API`、`2=Grok Build`（截图 48%/22% 量级） |
| `1.8` | 周期元数据 | 再嵌一套 start/end |
| trailers | `grpc-status: 0` | 成功 |

解析策略可直接参考 CodexBar `GrokWebBillingFetcher`：

- 扫 fixed32，取 path 末字段号=1 且 `0..100` 的 float 作为 percent。
- 扫 varint epoch（`1.7e9..2.1e9`），优先 path `[1,5,1]` 作为 reset。
- 若有周期但 percent 缺省，可按 0% 处理。

### 2. sub2api 路线（被动 rate-limit）

`Wei-Shaw/sub2api` 明确声明：

> xAI quota is passive. … records whitelisted xAI rate-limit headers … Before the first usable upstream response, dashboard shows quota as unknown.

实现要点：

- `backend/internal/pkg/xai/quota.go`：白名单解析
  - `x-ratelimit-limit/remaining/reset-requests|tokens`
  - `retry-after`
  - `xai-subscription-tier` / `x-subscription-tier`
  - `xai-entitlement-status` / `x-entitlement-status`
- `GrokQuotaService.ProbeUsage`：对 `POST {base}/responses` 发
  `{"model":"grok-4.3","input":".","max_output_tokens":1,"store":false}`，只为收集响应头。
- `ResetQuota`：**直接 501**——xAI 不暴露 OAuth 账号的订阅配额重置 API。
- **不调用** grok.com `GetGrokCreditsConfig`（README 也写了 out of scope：browser automation / grok web scraping）。

结论：sub2api 拿到的是 **API 网关 RPM/TPM 快照**，不是截图里的 Weekly SuperGrok credits。

### 3. CLIProxyAPI（业内常称 CPA）

`router-for-me/CLIProxyAPI`：

- 有完整 xAI OAuth（`internal/auth/xai`）与 Grok Build 多账号转发。
- 用量/配额在架构上主要是 **本地 usage statistics + 429/quota cooldown 调度**，不是 SuperGrok 周额度仪表盘。
- 生态里的 “Quota Inspector / Quotio” 等对 Codex/Claude 的 5h/7d 很强，对 Grok 更多是代理侧统计。
- **没有** 找到与 CodexBar 同级的 `GetGrokCreditsConfig` 实现。

### 4. 其它相关端点（本机探测）

| 端点 | 结果 |
|---|---|
| `GET https://management-api.x.ai/auth/teams` | 200，返回 teamId / tier 等，**无 usage %** |
| `management-api.x.ai/.../billing|usage|credits` | 404 / nginx 404 |
| `management-api.x.ai/auth/teams/{id}/api-keys` 等 | 403 `oauth2-auth-forbidden` |
| `https://api.x.ai/v1/usage` 等 | 无可用订阅用量 JSON |
| `grok agent stdio` `x.ai/billing` | 依赖本机 grok CLI 版本；CodexBar 记为常不可用 |

### 5. GrokGo 现状与建议实现

**已有**

- OAuth 权限已对齐 Grok Build（scope + `referrer=grok-build`）。
- 代理路径 `apply_rate_limit_headers` 镜像 requests limit/remaining（reset 多数为空）。
- SQLite `usage` 是 **本地请求日志成本估算**，不是上游订阅剩余。

**建议（实现方向，尚未编码）**

1. 新增 `quota` / `billing` 模块：对每个 enabled 账号用当前 access token 调 `GetGrokCreditsConfig`。
2. 解析 total percent、reset、product breakdown（id 1/2 先写死映射 API / Grok Build，保留 raw id）。
3. 缓存到 `Account` 或独立表：`quota_used_percent`、`quota_reset_at`、`quota_products`、`quota_fetched_at`。
4. UI Accounts / Overview 展示 SuperGrok 条；rate-limit remaining 单独标成 “API rate window”。
5. 刷新策略：打开页面 / 手动刷新 / 可选每 N 分钟；**不要**每条代理请求都打 billing。
6. 失败降级：billing 失败时仍显示 rate-limit 与本地 usage，避免整页挂。
7. 代理可选继续增强：记录 tokens 维度 header；429 时的 reset / retry-after。

## 相关页面

- [[modules/auth-oauth]]
- [[modules/usage-logging]]
- [[modules/routing]]
- [[../queries/account-quota-research]]

## 来源

- 本机实测：`~/.grok-go/auth.json` OAuth token → `GetGrokCreditsConfig` / `api.x.ai/v1/responses`
- CodexBar：`docs/grok.md`、`GrokWebBillingFetcher.swift`、`GrokRPCClient.swift`
- sub2api：`README.md` Grok 章节、`pkg/xai/quota.go`、`service/grok_quota_*.go`
- CLIProxyAPI：`internal/auth/xai`、README 生态说明
- 用户截图：Weekly SuperGrok Limit UI
