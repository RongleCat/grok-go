# Raw：账号用量调研来源指针

调研日期：2026-07-12
工作树：`../grok-go-usage-quota`（分支 `feat/account-quota-usage` @ `8f1a6e4`）

## 本地克隆（/tmp/quota-research）

- CodexBar: https://github.com/steipete/CodexBar
  - `docs/grok.md`
  - `Sources/CodexBarCore/Providers/Grok/GrokWebBillingFetcher.swift`
  - `Sources/CodexBarCore/Providers/Grok/GrokRPCClient.swift`
- sub2api: https://github.com/Wei-Shaw/sub2api
  - `README.md` § Grok / xAI OAuth Support
  - `backend/internal/pkg/xai/quota.go`
  - `backend/internal/service/grok_quota_service.go`
  - `backend/internal/service/grok_quota_fetcher.go`
- CLIProxyAPI (CPA): https://github.com/router-for-me/CLIProxyAPI
  - `internal/auth/xai/`
  - README 生态说明（Quota Inspector 等主要面向其它 provider）

## 本机实测样本

- OAuth store: `~/.grok-go/auth.json`（勿提交）
- 成功 billing 响应样例：`/tmp/gA1.bin`（调研时生成，不入库）
- API rate-limit 头：`POST https://api.x.ai/v1/responses` 返回 limit/remaining，无 reset
- 截图：用户提供的 grok.com Usage 面板（Weekly SuperGrok Limit）

## 关键端点

- `POST https://grok.com/grok_api_v2.GrokBuildBilling/GetGrokCreditsConfig`
- `POST https://api.x.ai/v1/responses`
- `GET https://management-api.x.ai/auth/teams`
