# GrokGo 项目 LLM Wiki

> 给后续任意 Agent / 人类接手用的项目知识库。  
> 先读本页与 [[wiki/index]]，再按任务跳到模块页。

## 30 秒理解

**GrokGo** 是本地桌面网关（Tauri 2 + React + Rust）：

1. 用 xAI OAuth 托管多个 Grok 账号；
2. 在本机启动 HTTP 网关（默认 `127.0.0.1:8787`）；
3. 对外提供 OpenAI / Responses 兼容 API + MCP 工具；
4. 让 Codex / Claude 等 Agent 用 Grok 做对话、X 搜索、出图、出视频。

配置与数据在 `~/.grok-go/`，**不在仓库内**。

## 接手必读顺序

1. 本文件 + [[wiki/index]]
2. [[wiki/syntheses/project-overview]]
3. [[wiki/syntheses/architecture]]
4. 按任务读模块：
   - 改代理逻辑 → [[wiki/modules/gateway]]、[[wiki/concepts/request-sanitize]]
   - 改登录 → [[wiki/modules/auth-oauth]]
   - 改负载均衡 → [[wiki/modules/routing]]
   - 改 MCP / 出图视频 → [[wiki/modules/mcp-tools]]、[[wiki/modules/media-artifacts]]
   - 改一键集成 → [[wiki/modules/integrations]]
   - 改 UI → [[wiki/modules/frontend-ui]]
5. 动手前看 [[wiki/playbooks/agent-onboarding]] 与 [[wiki/queries/faq]]

## 当前版本锚点

- 版本：`0.1.1`（`package.json` / `src-tauri/Cargo.toml` / `tauri.conf.json` 三处一致）
- 仓库：https://github.com/RongleCat/grok-go
- 作者 X：@cgnot996
- 产品前身：Grok Proxy（配置目录与 MCP id 已迁移到 grok-go，仍兼容旧标记）

## 目录地图

```text
llm-wiki/
  SCHEMA.md
  README.md
  raw/                 来源指针
  wiki/                编译后的知识
```

## 推荐提示词（复制给新 Agent）

```text
请先读 llm-wiki/README.md、llm-wiki/SCHEMA.md 和 llm-wiki/wiki/index.md。
本仓库是 GrokGo：本地 Grok 网关桌面应用。
改代码前先定位对应 wiki/modules 或 concepts 页；改完后回写相关 wiki 页与 wiki/log.md。
禁止提交 auth.json / 本地 token；MCP 工具参数以运行时 tools/list 或 ~/.grok-go/agents-guide.md 为准。
```
