# GrokGo 交接文档

**更新时间：** 2026-07-10  
**当前版本：** 0.1.0（可运行的第一版骨架）  
**交接对象：**
- **Grok**：继续完整开发 / 修 bug / 补能力
- **Codex**：负责验证是否可正常接入本地代理与 MCP

---

## 1. 项目目标

做一个跨平台 Tauri 桌面端，把 xAI OAuth 登录的 Grok 能力接到 Codex 和其他工具：

1. 本地 **Responses** 代理（Codex 主路径）
2. 本地 **OpenAI 兼容** 代理（其他工具）
3. 原生图片接口 + MCP 图片工具
4. MCP：`x_search` / image / video
5. 多账号托管 + 负载均衡
6. 请求日志 / token 统计 / 热力图
7. Codex MCP 自动注入/清理
8. 一键导入 CC Switch provider

设计文档：

- `docs/plans/2026-07-10-grok-go-desktop-design.md`

---

## 2. 当前实现状态

### 已实现（可用）

| 模块 | 状态 | 说明 |
|---|---|---|
| Tauri 2 项目脚手架 | 完成 | React + TS + Vite + Tailwind |
| 本地网关启动 | 完成 | App 启动时自动起 gateway |
| 端口自动递增 | 完成 | 首选 8787，冲突时递增 |
| 本地 bearer token | 完成 | 默认要求 token |
| Responses 代理 | 完成 | `POST /v1/responses` |
| Chat Completions 代理 | 完成 | `POST /v1/chat/completions` |
| Models | 完成 | `GET /v1/models` |
| Images 代理 | 完成 | generations / edits |
| Videos 代理 | 完成 | generations / edits |
| MCP endpoint | 完成 | `/mcp`，JSON-RPC 风格工具调用 |
| 多账号 OAuth PKCE | 完成 | 浏览器登录 + 本地 callback |
| Token refresh | 完成 | 过期/401 时刷新 |
| 模型映射 | 完成 | 未命中回退默认文本模型 |
| 请求日志 + 热力图 | 完成 | SQLite |
| Codex MCP 注入/清理 | 完成 | 仅管理 `mcp_servers.grok-go` |
| CC Switch provider 导入 | 完成 | 写入 `~/.cc-switch/cc-switch.db` 或导出 JSON |
| 控制台 UI | 完成 | Overview / Accounts / Mapping / Integrations / Usage / Logs / Settings |

### 已验证

```bash
# 后端编译
cd src-tauri && cargo check

# 前端构建
pnpm build

# 调试二进制可运行
src-tauri/target/debug/grok-go
# 健康检查成功
curl http://127.0.0.1:8787/health
# => running=true
```

### 已知问题 / 未完成

1. **dmg 打包失败**
   - `pnpm tauri build --debug` 时 app 本体成功
   - `bundle_dmg.sh` 失败
   - 不影响开发与本地调试

2. **OAuth 真实可用性待验证**
   - 当前复用公开 xAI OAuth client id：
     - `b1a00492-073a-47ea-816f-4c329264a828`
   - 需要真实账号登录验证 token 交换 / refresh / 上游 API 权限

3. **MCP 传输较简**
   - 当前是 HTTP JSON-RPC 简化实现
   - 未必覆盖 Codex 所有 MCP transport 细节（SSE/session/headers）
   - Codex 注入后若工具不可见，优先排查 MCP transport 兼容性

4. **MCP 鉴权**
   - 网关默认要求 bearer token
   - Codex MCP 配置目前只写了 `url`
   - 若 Codex 不带 Authorization，可能 401
   - 需要补：
     - MCP headers 支持
     - 或对 `/mcp` 放宽鉴权策略（仅 loopback）

5. **托盘 / 开机自启 / 关窗驻留**
   - 配置字段已有
   - 完整桌面行为未完全落地

6. **图片/视频产物本地缓存**
   - artifacts 目录已建
   - 自动下载落盘逻辑未完善

7. **空目录**
   - `src-tauri/src/state/` 为空占位，可删或后续使用

8. **计费仅为估算**
   - `usage.rs` 里 cost 是占位公式，不是官方账单

---

## 3. 目录结构

```text
grok-go/
├── docs/
│   ├── HANDOFF.md                              # 本交接文档
│   └── plans/2026-07-10-grok-go-desktop-design.md
├── src/                                        # 前端 React
│   ├── App.tsx
│   ├── main.tsx
│   ├── index.css
│   ├── components/
│   │   ├── layout.tsx
│   │   ├── heatmap.tsx
│   │   └── ui/                                 # shadcn 风格基础组件
│   ├── lib/
│   │   ├── api.ts                              # 前端 invoke API
│   │   └── utils.ts
│   └── pages/
│       ├── Overview.tsx
│       ├── Accounts.tsx
│       ├── Mapping.tsx
│       ├── Integrations.tsx
│       ├── Usage.tsx
│       ├── Logs.tsx
│       └── Settings.tsx
└── src-tauri/                                  # Rust / Tauri
    ├── Cargo.toml
    ├── tauri.conf.json
    ├── capabilities/default.json
    └── src/
        ├── lib.rs                              # app bootstrap
        ├── main.rs
        ├── commands.rs                         # Tauri commands
        ├── config.rs                           # config/auth schema
        ├── auth.rs                             # OAuth PKCE
        ├── router.rs                           # multi-account routing
        ├── usage.rs                            # sqlite logs/stats
        ├── integrations.rs                     # Codex MCP / CC Switch
        ├── paths.rs
        ├── error.rs
        └── gateway/
            ├── server.rs                       # axum routes
            └── proxy.rs                        # upstream proxy
```

---

## 4. 运行时与配置

### 配置目录

```text
~/.grok-go/
  config.json
  auth.json
  data.db
  artifacts/
  logs/
  backups/
```

Windows 对应：

```text
%USERPROFILE%\.grok-go\
```

### `config.json` 关键字段

- `preferredPort` / `actualPort`
- `lanEnabled`
- `requireToken`
- `localToken`
- `defaultModel`
- `defaultImageModel`
- `defaultVideoModel`
- `modelMappings`
- `routingStrategy`
- `autoInjectCodexMcp`
- `xaiClientId`
- `xaiBaseUrl`
- `oauthRedirectPort`

### 默认值

- preferred port: `8787`
- bind: `127.0.0.1`
- require token: `true`
- default model: `grok-4.5`
- default image model: `grok-imagine-image-quality`
- default video model: `grok-imagine-video`
- mapping 示例：
  - `gpt-5.6 -> grok-4.5`
  - `gpt-5.5 -> grok-4.5`

### 本地 API

| Method | Path | 用途 |
|---|---|---|
| GET | `/health` | 健康检查 |
| GET | `/v1/models` | 模型列表 |
| POST | `/v1/responses` | Codex 主路径 |
| POST | `/v1/chat/completions` | OpenAI 兼容 |
| POST | `/v1/images/generations` | 文生图 |
| POST | `/v1/images/edits` | 图编辑 |
| POST | `/v1/videos/generations` | 视频生成 |
| POST | `/v1/videos/edits` | 视频编辑 |
| ANY | `/mcp` | MCP 工具网关 |

鉴权：

```http
Authorization: Bearer <localToken>
```

`localToken` 在 Overview / Settings 可见。

### MCP 工具

- `x_search`
- `image_generate`
- `image_edit`
- `video_generate`
- `video_edit`

---

## 5. 开发启动

需要：

- Node.js + pnpm
- Rust stable
- macOS Command Line Tools / Windows MSVC 构建环境

可选代理：

```bash
export all_proxy=http://127.0.0.1:7890
export https_proxy=http://127.0.0.1:7890
export http_proxy=http://127.0.0.1:7890
```

安装与开发：

```bash
cd /Users/ronglecat/Documents/workspace/tools/grok-go
pnpm install
pnpm tauri dev
```

仅前端：

```bash
pnpm dev
```

仅检查后端：

```bash
cd src-tauri
cargo check
```

已有调试二进制：

```bash
./src-tauri/target/debug/grok-go
```

App bundle：

```text
src-tauri/target/debug/bundle/macos/GrokGo.app
```

---

## 6. 给 Grok 的继续开发清单

按优先级：

### P0：打通 Codex 真实接入

1. **验证 OAuth 登录闭环**
   - Accounts 页登录
   - `auth.json` 写入 access/refresh token
   - 上游 `/v1/responses` 成功

2. **修 MCP 与 Codex 兼容** ✅（2026-07-11）
   - 根因：`notifications/initialized` 误返回 JSON-RPC `result`，Codex streamable HTTP 反序列化失败
   - 修复：notification 返回 `202 Accepted` + 空 body；`initialize` 协商 `2025-06-18`/`2024-11-05`
   - 验证：`cargo test mcp_handshake`；本地 handshake → tools/list 返回 6 个工具
   - 确认 Codex 是否要求 SSE / streamable HTTP
   - 给 MCP 增加 headers 注入能力：
     ```toml
     [mcp_servers.grok-go]
     url = "http://127.0.0.1:8787/mcp"
     # headers / auth 视 Codex 支持字段补充
     ```
   - 或 loopback 下 `/mcp` 免 token（需明确安全边界）

3. **修 Codex provider 使用路径**
   - 不默认改全局 provider
   - 保证 Overview 复制的 snippet 可直接用
   - CC Switch 导入后可切换到 GrokGo

### P1：代理正确性

1. streaming 透传稳定性
2. 模型映射日志更清晰
3. 上游错误体尽量原样返回
4. 图片/视频响应本地缓存到 `artifacts/`
5. LAN 模式显示真实局域网 IP，并强制 token

### P2：产品完善

1. 托盘、关窗最小化、开机自启真正落地
2. 多账号路由策略可视化
3. 用量面板按账号/模型聚合
4. dmg/msi 打包修复
5. Windows 完整验证

### P3：质量

1. 给 `resolve_model` / router / integrations 补单元测试
2. 给 gateway 加集成测试（mock upstream）
3. 日志脱敏检查
4. 配置导入导出 UI

### 开发约束

- UI：优先复用 `src/components/ui/*`，保持 shadcn 风格
- 设计：遵循 emilkowalski design skills 的克制动画与工具台审美
- 不要把 token 存系统钥匙串；保持可迁移配置文件
- Codex MCP 注入只能动 `mcp_servers.grok-go`
- 不要默认覆盖用户现有 Codex provider

---

## 7. 给 Codex 的接入测试手册

目标：验证 Codex 是否能通过本代理正常使用 Grok。

### 7.1 前置

1. 启动 GrokGo：
   ```bash
   pnpm tauri dev
   # 或
   ./src-tauri/target/debug/grok-go
   ```
2. 打开 App，确认 Overview 显示 gateway `online`
3. 在 Accounts 登录至少一个 xAI 账号
4. 记录：
   - actual port（默认 8787）
   - base URL
   - local token

健康检查：

```bash
curl -s http://127.0.0.1:8787/health
```

带 token 测 models：

```bash
curl -s http://127.0.0.1:8787/v1/models \
  -H "Authorization: Bearer <localToken>"
```

### 7.2 方案 A：直接改 Codex provider（手动）

编辑 `~/.codex/config.toml`，新增或切换到：

```toml
model_provider = "grok-go"
model = "grok-4.5"

[model_providers.grok-go]
name = "grok-go"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:8787/v1"
experimental_bearer_token = "<localToken>"
```

注意：

- 必须是 `wire_api = "responses"`
- `base_url` 要带 `/v1`
- token 必须与 GrokGo 的 `localToken` 一致

可选模型映射：

- Codex 里继续选 `gpt-5.6`
- 代理会按 `modelMappings` 映射到 `grok-4.5`

### 7.3 方案 B：通过 CC Switch

1. App → Integrations → **Import provider into CC Switch**
2. 打开 CC Switch
3. 找到 `GrokGo` provider
4. 切换为当前 Codex provider
5. 重启 / 刷新 Codex

### 7.4 MCP 测试

App → Integrations → 打开 **Codex MCP inject**

会写入：

```toml
[mcp_servers.grok-go]
url = "http://127.0.0.1:8787/mcp"
```

关闭开关应只删除这一项，不影响其他 MCP。

Codex 侧验证：

1. MCP 服务器是否加载成功
2. 是否可见工具：
   - `x_search`
   - `image_generate`
   - `image_edit`
   - `video_generate`
   - `video_edit`
3. 调用 `x_search` 是否返回结果

如果 MCP 启动失败，优先检查：

- URL 端口是否是 actual port
- 是否缺 Authorization
- Codex 是否要求 SSE MCP 而不是普通 HTTP JSON-RPC

### 7.5 Codex 验收用例

#### Case 1：普通对话

Prompt：

```text
用一句话介绍你自己，并说明当前模型名。
```

期望：

- Codex 正常流式/非流式返回
- GrokGo Logs 出现 `/v1/responses`
- status 200

#### Case 2：模型映射

1. Mapping 配置：`gpt-5.6 -> grok-4.5`
2. Codex 选择 `gpt-5.6`
3. 发起请求

期望：

- Logs 中 `requestedModel=gpt-5.6`
- `resolvedModel=grok-4.5`

#### Case 3：图片生成

Prompt：

```text
生成一张极简黑白图标风格的狐狸头像。
```

期望：

- Codex 走原生 image generation 路径
- 代理命中 `/v1/images/generations`
- 返回图片 URL 或可用结果

#### Case 4：x_search

Prompt：

```text
用 x_search 搜索 xAI 最近关于 Grok 的讨论，并给出 3 条链接摘要。
```

期望：

- MCP 工具被调用，或至少工具可见
- 返回含 X 链接/摘要的结果
- Logs 有 mcp / responses 记录

#### Case 5：失败可观测

1. 故意使用错误 token
2. 请求 `/v1/responses`

期望：

- 401
- App Logs 可见失败记录
- 不影响配置文件整体损坏

### 7.6 Codex 侧快速自检命令

```bash
# 1. health
curl -s http://127.0.0.1:8787/health | jq .

# 2. models
curl -s http://127.0.0.1:8787/v1/models \
  -H "Authorization: Bearer <localToken>" | jq .

# 3. responses smoke
curl -s http://127.0.0.1:8787/v1/responses \
  -H "Authorization: Bearer <localToken>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5.6",
    "input": "ping"
  }' | jq .
```

若第 3 步成功，说明：

- 本地鉴权 OK
- 模型映射 OK
- 上游 OAuth token OK
- Responses 路径 OK

这时 Codex 只要 provider 配对该就能接上。

---

## 8. 关键代码入口

| 想改什么 | 去哪里 |
|---|---|
| 增加/修改 Tauri 命令 | `src-tauri/src/commands.rs` |
| 路由/端口/鉴权/MCP | `src-tauri/src/gateway/server.rs` |
| 上游转发/模型映射注入 | `src-tauri/src/gateway/proxy.rs` |
| OAuth 登录 | `src-tauri/src/auth.rs` |
| 多账号选择策略 | `src-tauri/src/router.rs` |
| 配置结构 | `src-tauri/src/config.rs` |
| 日志统计 | `src-tauri/src/usage.rs` |
| Codex MCP / CC Switch | `src-tauri/src/integrations.rs` |
| 前端 API | `src/lib/api.ts` |
| 页面 | `src/pages/*` |

---

## 9. 数据流（简图）

```text
Codex
  |  Responses + Bearer localToken
  v
GrokGo Gateway (:8787/v1/responses)
  |  model mapping
  |  pick account + refresh token
  v
xAI API (https://api.x.ai/v1/...)
  |
  v
request_logs (SQLite) + UI Usage/Logs
```

MCP：

```text
Codex MCP client
  -> http://127.0.0.1:8787/mcp
  -> tools/call
  -> xAI responses/images/videos
```

---

## 10. 安全注意

- `~/.grok-go/auth.json` 含 OAuth token，可迁移但不要公开分享
- `localToken` 可访问全部本地代理能力
- LAN 开启后不要关闭 token
- Codex/CC Switch 导入会写入本地 token，注意机器边界

---

## 11. 建议的下一步分工

### Grok 继续开发

1. 真账号打通 OAuth + Responses
2. 修 Codex MCP 兼容与鉴权
3. 补 streaming / 错误处理 / artifacts
4. 修打包
5. Windows 验证

### Codex 负责验收

1. provider 接入对话
2. 模型映射
3. 图片生成
4. MCP 工具可见与调用
5. 给出失败日志与复现步骤

---

## 12. 一句话现状

**项目已经是“可启动的桌面代理控制台 + 本地网关骨架”，核心路由/代理/MCP/统计都在；下一步关键不是重新搭架构，而是用真实 xAI 登录把 Codex 端到端打通，并修 MCP 鉴权/传输兼容问题。**
