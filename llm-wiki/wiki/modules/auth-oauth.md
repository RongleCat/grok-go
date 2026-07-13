# 模块：Auth / OAuth

## 结论

`auth.rs` 实现 xAI OIDC 登录（PKCE）。浏览器授权回调到本机 `http://127.0.0.1:56121/callback`，换 token 后写入 `auth.json`。代理热路径会 `ensure_fresh_token` 自动 refresh。

## 流程

1. UI 调 `start_oauth_login`
2. 拉取 `https://auth.x.ai/.well-known/openid-configuration`
3. 确保 callback 端口监听
4. 创建/复用账号记录（清理无 token 占位账号）
5. 生成 PKCE + state，拼 authorize URL
6. 打开浏览器；callback 校验 state，code → token
7. 可选 userinfo 填充 email/name
8. `save_auth` 落盘（unix 权限 0600）

## 重要约束

- redirect_uri 必须与公开 client 的 allowlist 一致：`http://127.0.0.1:56121/callback`
- 默认使用 hermes/grok-cli 系 public client id（见 `AppConfig` 默认值，源码路径 `config.rs`）
- scope 含：`openid profile email offline_access grok-cli:access api:access conversations:read conversations:write`
- authorize 参数含 `referrer=grok-build` 以对齐 Grok Build 权限面
- 未完成 OAuth 15 分钟清理 pending；登录有全局 gate 防连点堆账号

## 账号健康字段（Account）

- `enabled` / `weight`
- `access_token` / `refresh_token` / `expires_at`
- `health`: healthy | degraded | cooldown | disabled
- `cooldown_until`、`consecutive_failures`
- 上游 rate limit 头镜像字段
- `last_upstream_error` 供 UI 诊断
- `supports_image` / `supports_video`（默认 true）：普通文本号可关掉，媒体路由会跳过

## 批量导入（CPA / sub2api / 卡网 SSO）

解析实现：`account_import.rs`；命令：`import_accounts`。

| 格式 | 说明 |
|---|---|
| 纯 refresh_token 列表 | 每行一个（sub2api 批量 RT / 卡网 SSO 粘贴） |
| CPA `xai-*.json` | `type=xai` + access/refresh token 等 |
| JSON 数组 / NDJSON | 多个 CPA 文件合并 |
| sub2api credentials | `platform=grok` + 嵌套 `credentials` |
| GrokGo `auth.json` | `{ accounts: [...] }` |
| SSO 包装对象 | 递归抽取含 `refresh_token`/`access_token` 的字段 |

导入选项：`weight`、`supportsImage`/`supportsVideo`、`skipDuplicates`、`validateRefresh`（默认 true，会调 token endpoint 校验 RT 并补全 email）。

批量管理命令：

- `batch_delete_accounts(ids)`
- `batch_patch_accounts(ids, { enabled, weight, supportsImage, supportsVideo, clearCooldown })`

## 相关页面

- [[routing]]
- [[config-runtime]]
- [[frontend-ui]]
- [[../playbooks/debug-checklist]]

## 来源

- `src-tauri/src/auth.rs`
- `src-tauri/src/account_import.rs`
- `src-tauri/src/config.rs`（Account / AuthStore）
- `src-tauri/src/commands.rs`（start_oauth_login / import_accounts / batch_*）
- `src-tauri/src/router.rs`（batch_update_accounts）

## SuperGrok 周配额

- 与 rate-limit 头分离；见 [[../concepts/account-quota]]
- 实现：`src-tauri/src/quota.rs`，写入 `Account.quota`

