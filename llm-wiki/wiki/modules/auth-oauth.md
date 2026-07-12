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

## 相关页面

- [[routing]]
- [[config-runtime]]
- [[../playbooks/debug-checklist]]

## 来源

- `src-tauri/src/auth.rs`
- `src-tauri/src/config.rs`（Account / AuthStore）
- `src-tauri/src/commands.rs`（start_oauth_login）
