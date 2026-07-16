<p align="center">
  <img src="assets/logo.png" alt="GrokGo logo" width="120" height="120" />
</p>

<h1 align="center">GrokGo</h1>

<p align="center"><strong>本地 Grok / xAI 网关桌面应用</strong></p>
<p align="center"><em>Grok, ready to go for Codex</em></p>

<p align="center">
  <a href="./README.md">中文</a> ·
  <a href="./README_EN.md">English</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/note-use%20with%20care-yellow" alt="Use with care" />
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License" /></a>
  <a href="https://github.com/RongleCat/grok-go/stargazers"><img src="https://img.shields.io/github/stars/RongleCat/grok-go?style=social" alt="GitHub stars" /></a>
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey" alt="Platforms" />
  <img src="https://img.shields.io/badge/Tauri-2-orange" alt="Tauri 2" />
</p>

<p align="center">
  在 X 关注作者
  <a href="https://x.com/cgnot996"><strong>铁柱AGI@cgnot996</strong></a>
  · 仓库
  <a href="https://github.com/RongleCat/grok-go">RongleCat/grok-go</a>
</p>

---

> [!WARNING]
> ## 使用提示
>
> 官方路径通常不鼓励用第三方本地网关这种方式接入；请自行评估风险，**不要当作长期生产方案**。
>
> 就当前实现而言，**API 模式可以正常使用**，适合个人场景下的**轻度使用**。设置里请保持默认的 **API** 渠道；**Grok Build** 会话面仍有更高账号风险，不建议日常打开。
>
> 本仓库维护节奏会放缓，兼容性与上游策略可能随时变化，请以实际连通为准。

---

## 目录

1. [简介](#简介)
2. [功能特性](#功能特性)
3. [界面预览](#界面预览)
4. [安装与使用](#安装与使用)
5. [接入 Codex / 客户端](#接入-codex--客户端)
6. [渠道选择（重要）](#渠道选择重要)
7. [API 端点](#api-端点)
8. [配置目录](#配置目录)
9. [macOS 无法打开 / 提示已损坏](#macos-无法打开--提示已损坏)
10. [开发与构建](#开发与构建)
11. [文档与贡献](#文档与贡献)

---

## 简介

把 **Grok / xAI** 接到 **Codex**、OpenAI 兼容客户端或其他 Agent 时，通常要自己处理 OAuth、本地代理、MCP、多账号路由和用量统计。

**GrokGo** 把这些能力收成一个本地桌面网关：

1. 安装并启动  
2. 登录或导入账号  
3. 复制 Base URL + Token  
4. 在客户端里填入即可使用  

---

## 功能特性

| 类别 | 说明 |
|------|------|
| **协议兼容** | `/v1/responses`、`/v1/chat/completions`、`/v1/models` |
| **MCP 工具** | `x_search`、图片生成/编辑、视频生成/编辑 |
| **多账号** | OAuth 托管、加权负载均衡、自动 refresh |
| **批量导入** | CPA `xai-*.json` / sub2api RT / 卡密 SSO→OAuth / GrokGo `auth.json` |
| **媒体能力** | 图片 / 视频走同一鉴权网关，产物在 `~/.grok-go/artifacts/` |
| **用量可视** | 请求日志（命中账号）、Token 统计、SuperGrok 周额度 + API 限额、热力图 |
| **一键集成** | Codex MCP 注入、`mcp_servers.grok-go`、CC Switch provider 导入 |
| **安全访问** | 本地 bearer token；可选局域网访问 |

---

## 界面预览

> 截图来自当前开发版界面（账号邮箱已脱敏）。

| 概览 | 账号 |
|:---:|:---:|
| ![Overview](assets/screenshots/overview.png) | ![Accounts](assets/screenshots/accounts.png) |

| 集成 | 用量 |
|:---:|:---:|
| ![Integrations](assets/screenshots/integrations.png) | ![Usage](assets/screenshots/usage.png) |

---

## 安装与使用

### 1. 下载

> 若仍要试用：请用 **API 模式**，轻度使用即可。

从 [Releases](https://github.com/RongleCat/grok-go/releases) 下载对应平台安装包：

| 平台 | 文件 |
|------|------|
| macOS Apple Silicon | `GrokGo_*_aarch64.dmg` |
| macOS Intel | `GrokGo_*_x64.dmg` |
| Windows x64 | `.msi` / `.exe` |

### 2. 首次使用

1. 启动 GrokGo，确认 **概览** 页网关为「运行中」  
2. 在 **账号** 页登录，或批量导入账号  
3. 在 **概览** 复制：  
   - Base URL：`http://127.0.0.1:<port>/v1`  
   - Local Token  
4. （可选）在 **集成** 页一键注入 Codex MCP / 导入 CC Switch  

默认端口 **8787**，占用时会自动递增。

---

## 接入 Codex / 客户端

### 手动配置

1. 启动 GrokGo，在 **概览** 复制 Base URL 与 Local Token  
2. 客户端使用 **Responses API**（或 OpenAI Chat Completions 兼容模式）  
3. 填写：  
   - Base URL：`http://127.0.0.1:<port>/v1`  
   - Authorization：`Bearer <localToken>`  

### 一键注入 MCP（Codex）

在 **集成** 页开启 MCP 注入后，效果类似：

```toml
[mcp_servers.grok-go]
url = "http://127.0.0.1:<port>/mcp"

[mcp_servers.grok-go.http_headers]
Authorization = "Bearer <localToken>"
```

---

## API 端点

应用启动后，本地网关默认提供：

| 用途 | 地址 |
|------|------|
| Base | `http://127.0.0.1:<port>/v1` |
| Responses | `POST /v1/responses` |
| Chat Completions | `POST /v1/chat/completions` |
| Images | `POST /v1/images/generations`、`POST /v1/images/edits` |
| MCP | `http://127.0.0.1:<port>/mcp` |

---

## 配置目录

```text
~/.grok-go/
  config.json      # 网关 / 端口 / 集成等配置
  auth.json        # 账号与 token（请勿提交）
  data.db          # 用量与日志
  artifacts/       # 媒体产物
  backups/         # 备份
  agents-guide.md  # 运行时 MCP 工具指引（按启用工具生成）
```

---

## macOS 无法打开 / 提示已损坏

当前 Release **未做 Apple 公证**（需付费开发者账号）。从 GitHub 下载后，Gatekeeper 可能提示「已损坏」「无法验证开发者」等，属预期行为。

**推荐处理：**

```bash
# 将 App 拖到「应用程序」后执行
xattr -cr /Applications/GrokGo.app
open /Applications/GrokGo.app
```

**其他方式：**

- Finder 中 **右键** App → **打开** → 再次确认打开  
- **系统设置 → 隐私与安全性** → 对拦截项点 **仍要打开**  

请仅从本仓库官方 [Releases](https://github.com/RongleCat/grok-go/releases) 下载。配置签名与公证后，上述步骤将不再需要。

---

## 开发与构建

```bash
# 依赖
pnpm install

# 开发（前端 + Tauri）
pnpm tauri dev

# 仅前端
pnpm dev:ui

# 生产构建
pnpm tauri build
```

交叉编译、发版与可选签名见 [docs/BUILD.md](./docs/BUILD.md)。

**技术栈：** Tauri 2 + Rust · React + TypeScript + Vite · Tailwind CSS

---

## 文档与贡献

> 维护节奏放缓；欢迎反馈，但请理解响应可能较慢。


| 对象 | 入口 |
|------|------|
| AI Agent / 源码接手 | [`llm-wiki/README.md`](./llm-wiki/README.md) |
| 贡献指南 | [CONTRIBUTING.md](./CONTRIBUTING.md) |
| 行为准则 | [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md) |
| 安全披露 | [SECURITY.md](./SECURITY.md) |

欢迎 Issue 与 PR。

## License

[MIT](./LICENSE) © RongleCat

---

<p align="center">
  如果 GrokGo 对你有帮助，请点个 Star，并在 X 关注
  <a href="https://x.com/cgnot996">铁柱AGI@cgnot996</a>
</p>
