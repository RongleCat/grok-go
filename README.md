<p align="center">
  <img src="assets/logo.png" alt="GrokGo logo" width="120" height="120" />
</p>

<h1 align="center">GrokGo</h1>

<p align="center"><strong>本地 Grok 网关，即开即用</strong></p>
<p align="center"><em>Grok, ready to go for Codex</em></p>

<p align="center">
  <a href="./README.md">中文</a> ·
  <a href="./README_EN.md">English</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License" /></a>
  <a href="https://github.com/RongleCat/grok-go/stargazers"><img src="https://img.shields.io/github/stars/RongleCat/grok-go?style=social" alt="GitHub stars" /></a>
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey" alt="Platforms" />
  <img src="https://img.shields.io/badge/Tauri-2-orange" alt="Tauri 2" />
</p>

<p align="center">
  在 X 关注作者
  <a href="https://x.com/cgnot996"><strong>@cgnot996</strong></a>
  · 仓库
  <a href="https://github.com/RongleCat/grok-go">RongleCat/grok-go</a>
</p>

---

## 为什么需要 GrokGo？

想把 **Grok / xAI** 接到 **Codex** 或其他 AI 工具时，通常要自己处理 OAuth、本地代理、MCP、多账号与用量。  
**GrokGo** 把这些打包成一个即开即用的桌面网关：装好登录，填地址就能用。

## 功能亮点

- **Responses + OpenAI 兼容**：`/v1/responses`、`/v1/chat/completions`、`/v1/models`
- **MCP 工具**：`x_search`、图片生成/编辑、视频生成/编辑
- **多账号 OAuth**：托管多个账号，加权负载均衡，自动 refresh
- **原生媒体接口**：图片 / 视频走同一鉴权网关，产物落在 `~/.grok-go/artifacts/`
- **用量可视**：请求日志、Token 统计、GitHub 风格热力图
- **Codex / CC Switch**：一键注入 `mcp_servers.grok-go`，一键导入 provider
- **局域网可选访问**：本地 bearer token 保护

## 界面预览

| 概览 | 账号 |
|:---:|:---:|
| ![Overview](assets/screenshots/overview.png) | ![Accounts](assets/screenshots/accounts.png) |

| 集成 | 用量 |
|:---:|:---:|
| ![Integrations](assets/screenshots/integrations.png) | ![Usage](assets/screenshots/usage.png) |

## 快速开始

### 下载安装

从 [Releases](https://github.com/RongleCat/grok-go/releases) 下载对应平台安装包：

| 平台 | 文件 |
|------|------|
| macOS Apple Silicon | `GrokGo_*_aarch64.dmg` |
| macOS Intel | `GrokGo_*_x64.dmg` |
| Windows x64 | `.msi` / `.exe` 安装包 |

### 开发

```bash
pnpm install
pnpm tauri dev
```

仅前端：

```bash
pnpm dev:ui
```

### 构建

```bash
pnpm tauri build
```

更完整的交叉编译与发布说明见 [docs/BUILD.md](./docs/BUILD.md)。

## macOS：提示「已损坏」/「无法验证开发者」/「不安全」

当前 Release 安装包**未做 Apple 公证**（需要付费开发者账号）。从 GitHub 下载后，macOS Gatekeeper 常会拦截，属于预期行为，可按下面处理：

### 方法一：移除隔离属性（推荐）

1. 将 `GrokGo.app` 拖到「应用程序」
2. 打开「终端」，执行（路径按实际调整）：

```bash
# 若装在应用程序
xattr -cr /Applications/GrokGo.app

# 或针对 DMG 内 / 下载目录中的 app
xattr -cr ~/Downloads/GrokGo.app
```

3. 再双击打开

### 方法二：右键打开

1. 在 Finder 中 **右键**（或 Control + 点击）`GrokGo.app`
2. 选择 **打开**
3. 在弹窗中再次点 **打开**

> 仅「双击」可能仍被拦截；必须用右键菜单里的「打开」。

### 方法三：系统设置放行

1. 打开 **系统设置 → 隐私与安全性**
2. 若看到「已阻止使用 GrokGo…」，点击 **仍要打开**
3. 按提示输入密码确认

### 仍打不开时

```bash
# 确认隔离标记是否已清掉（无 com.apple.quarantine 即正常）
xattr -l /Applications/GrokGo.app

# 再清一次并尝试启动
xattr -cr /Applications/GrokGo.app
open /Applications/GrokGo.app
```

**说明：** 这些步骤只是绕过「未签名/未公证」的系统保护，请仅从本仓库官方 [Releases](https://github.com/RongleCat/grok-go/releases) 下载。后续若配置了 Apple 签名与公证，上述提示会消失。

## 接入 Codex

1. 启动 GrokGo，在 **概览** 页复制：
   - Base URL：`http://127.0.0.1:<port>/v1`
   - Local Token
2. 在 Codex 中使用 Responses API，并填写上述 base URL + bearer token
3. 可选：在 **集成** 页一键注入 MCP：

```toml
[mcp_servers.grok-go]
url = "http://127.0.0.1:<port>/mcp"

[mcp_servers.grok-go.http_headers]
Authorization = "Bearer <localToken>"
```

首选端口为 **8787**，冲突时自动递增。

## 默认端点

应用启动后：

| 用途 | 地址 |
|------|------|
| Base | `http://127.0.0.1:<port>/v1` |
| Responses | `POST /v1/responses` |
| Chat Completions | `POST /v1/chat/completions` |
| Images | `POST /v1/images/generations`, `POST /v1/images/edits` |
| MCP | `http://127.0.0.1:<port>/mcp` |

## 配置目录

```text
~/.grok-go/
  config.json
  auth.json
  data.db
  artifacts/
  backups/
```

## 技术栈

- Tauri 2 + Rust
- React + TypeScript + Vite
- Tailwind CSS

## 贡献

欢迎 Issue 与 PR，详见 [CONTRIBUTING.md](./CONTRIBUTING.md)。  
行为准则：[CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md)  
安全披露：[SECURITY.md](./SECURITY.md)

## License

[MIT](./LICENSE) © RongleCat

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=RongleCat/grok-go&type=Date)](https://star-history.com/#RongleCat/grok-go&Date)

---

<p align="center">
  如果 GrokGo 对你有帮助，请点个 Star，并在 X 关注
  <a href="https://x.com/cgnot996">@cgnot996</a>
</p>
