# GrokGo Wiki 索引

本页是 Agent 召回入口。每条都是一句话摘要。

## 先读

- [[syntheses/project-overview]]：产品是什么、解决什么、不做什么
- [[syntheses/architecture]]：进程内架构、数据目录、请求主路径
- [[playbooks/agent-onboarding]]：新 Agent 接手检查清单
- [[queries/faq]]：高频问题
- [[queries/account-quota-research]]：反代项目如何获取用量/刷新时间（调研结论）
- [[queries/proxy-cache-routing-research]]：sub2api/CPA 缓存省量与流量分配策略（可借鉴清单）

## 模块

- [[modules/gateway]]：Axum 本地网关、HTTP 端点、MCP endpoint
- [[modules/auth-oauth]]：xAI OIDC 登录、refresh、callback
- [[modules/routing]]：多账号选号、cooldown、同请求 failover
- [[modules/mcp-tools]]：MCP 工具目录与调用链路
- [[modules/media-artifacts]]：图片/视频落盘与 markdown 返回
- [[modules/integrations]]：Codex MCP / AGENTS 指引 / CC Switch / Grok Build
- [[modules/frontend-ui]]：React 页面与 Tauri command 边界
- [[modules/usage-logging]]：请求日志、热力图、用量汇总
- [[modules/config-runtime]]：配置字段、默认值、路径

## 概念

- [[concepts/model-mapping]]：请求模型名如何解析到 xAI 模型
- [[concepts/request-sanitize]]：Codex Responses payload 清洗与回写
- [[concepts/payload-optimize]]：多轮大文件/图片 token 膨胀抑制与 Files `file_id` 分流
- [[concepts/account-quota]]：SuperGrok 周配额 vs API rate-limit 头，如何取剩余用量与重置时间

## 操作手册

- [[playbooks/dev-and-release]]：开发、构建、发版
- [[playbooks/debug-checklist]]：常见故障排查顺序

## 维护

- [[log]]：wiki 变更日志
- 规则见 `../SCHEMA.md`、入口见 `../README.md`
