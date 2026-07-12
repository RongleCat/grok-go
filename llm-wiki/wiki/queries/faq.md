# 高频问答

## GrokGo 是什么？

本地 Grok 网关桌面应用：OAuth 多账号 + OpenAI/Responses 兼容 API + MCP 媒体工具。详见 [[../syntheses/project-overview]]。

## 和直接调 xAI API 有何不同？

多了：本地 token 保护、多账号路由与 failover、Codex payload 兼容、MCP 工具、媒体落盘、用量 UI、一键注入配置。

## 默认端口？

首选 8787；占用时自动递增，以 Overview / `actualPort` 为准。

## 配置在哪？

`~/.grok-go/`。仓库里没有用户 token。

## MCP 工具参数以谁为准？

运行时 `tools/list`。其次 `~/.grok-go/agents-guide.md`。不要靠猜或翻陈旧文档。

## 为什么媒体必须本地路径？

Codex 桌面用绝对路径 markdown 内联渲染；CDN 链接不稳定且不利于二次编辑链路。

## 品牌从 Grok Proxy 改名后要注意什么？

- 目录：`~/.grok-go`
- MCP id：`grok-go`（仍识别 `grok-proxy`）
- AGENTS 标记兼容旧 `grok-proxy` 注释块

## 能在无 UI 时用吗？

可以：应用启动后网关常驻；也可用 HTTP 直接打 `/v1/*`。但账号 OAuth 仍需桌面流程完成登录。

## 相关页面

- [[../playbooks/debug-checklist]]
- [[../modules/mcp-tools]]
- [[../modules/integrations]]
