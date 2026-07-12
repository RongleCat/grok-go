# Query：各反代如何获取 Grok 剩余用量与刷新时间？

## 问题

授权了更广 OAuth 权限后，如何把账号 **剩余用量 / 重置时间** 弄到 GrokGo？Grok Build、CPA、sub2api 等项目怎么做？

## 答案

### 一句话

- **截图那种 Weekly SuperGrok Limit + 重置时间**：走 `grok.com` 的 `GrokBuildBilling/GetGrokCreditsConfig`（CodexBar / Grok Build 路线）。
- **API RPM/TPM remaining**：读 `api.x.ai` 的 `x-ratelimit-*` 响应头（sub2api / GrokGo 已有被动路径）。
- **两套不要混**：480 remaining requests ≠ 30% credits left。

### 对照表

| 项目 | 订阅周配额 % + reset | rate-limit remaining | 备注 |
|---|---|---|---|
| Grok Build UI / CLI `/usage` | 是 | 间接 | 产品真相源 |
| CodexBar | 是（gRPC-web；CLI RPC 优先但常不可用） | 否 | 解析逻辑最完整 |
| sub2api | 否 | 是（被动 + probe） | 明确不爬 grok.com |
| CLIProxyAPI（CPA） | 未见官方订阅额度抓取 | 本地统计 + 429 cooldown | 多账号转发为主 |
| GrokGo（main / 本分支起点） | 否 | 部分（requests 头） | 本地 SQLite 只是自用日志 |

### 可执行调用（已用本机 token 打通）

```http
POST https://grok.com/grok_api_v2.GrokBuildBilling/GetGrokCreditsConfig
Authorization: Bearer <oauth_access_token>
Content-Type: application/grpc-web+proto
x-grpc-web: 1
x-user-agent: connect-es/2.1.1
Origin: https://grok.com
Referer: https://grok.com/?_s=usage

<body> 5 bytes: 00 00 00 00 00
```

成功时 body 为 gRPC-web frame + `grpc-status:0` trailer；内含 float 百分比与 Timestamp reset。

### 推荐 GrokGo 落地顺序

1. 账号级 `fetch_account_quota(token)` → 解析 percent / reset / products。
2. 写入 auth 旁路缓存或 Account 字段，UI 展示。
3. 保留现有 rate-limit header 作为调度辅助，文案区分 “订阅额度” vs “请求速率”。
4. 不要依赖 management-api.x.ai（无用量；多数 billing 路径 404/403）。

## 相关页面

- [[../concepts/account-quota]]

## 来源

- 2026-07-12 调研分支 `feat/account-quota-usage` 实测与源码对照
