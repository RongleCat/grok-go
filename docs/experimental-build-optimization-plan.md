# 仿冒 Grok Build 综合优化方案

> 日期：2026-07-16  
> 输入：  
> 1. Codex 体验报告 `Agent 实验室/grok-4.5-仿冒-grok-build-体验与优化点.md`  
> 2. Claude Code 缺陷清单 `docs/claude-code-compat-defects.md`  
> 3. WorkBuddy 体验报告 `WorkBuddy/.../grok-build-compat-feedback.md`  
> 范围：可在 **GrokGo 代码库**内落地的改动（不含改 Codex/Claude Code/WorkBuddy 上游产品）  
> 目标：仿冒 Build 保额度与会话面的同时，**工具可用、错误可读、长任务不假失败、多客户端主循环稳**

---

## 1. 三端共性诊断

| 共性痛点 | Codex | Claude Code | WorkBuddy | 根因抽象 |
|---|---|---|---|---|
| 能力「有」但「调不到」 | MCP 连上未进本轮 tools | — | ToolSearch 两跳 | 工具暴露层 / 注入策略 |
| 文档规则 > 运行时能力 | agents-guide 绝对禁止降级 | Claude 预期 vs Grok 行为 | 能力面分裂 | 决策树与契约说明 |
| 长任务假失败 | image/video 旁路重 | 图像尺寸 400 | 视频 MCP 超时但产物已生成 | 同步 RPC 套异步 job |
| 错误语义脏 | unsupported / 代理分裂 | 上游错误不够 Claude 化 | aborted / ECONNREFUSED | 分层 error code |
| 延迟 / thinking | 空完成 vs TTFT | thinking 透出拖 TTFT | 首包 system 过肥 | 可配置透出 + 强制非流式策略 |
| 双平面心智 | chat=build / media=console | Chat ⇄ Messages 债 | 同 | 可视化 + 文档表 |
| 观测不足 | plane 不可见 | convert/upstream 分层 | 网关挂了才知道 | 埋点 + 响应头 |

**产品边界（写进方案前提）：**

```text
仿冒 Grok Build ≠ 100% Grok Build TUI 同构
= SuperGrok 会话面（cli-chat-proxy）
+ 本地网关能力（MCP / 媒体 / 续跑 / 多账号）
+ 各客户端原生协议壳（Responses / Messages / Chat）
```

当前最大落差（Codex 报告原话）：

> 已经享受了 Build 的「会话面」约束，却没有补回 Codex 需要的「工具面」厚度。

---

## 2. 已完成（不必重复开干）

| 项 | 状态 | 提交/模块 |
|---|---|---|
| 仿冒开关 + 官方头对齐 | 已做 | `build_plane_route` |
| experimental 开 empty-completion | 已做 | `apply_empty_completion_recovery` |
| 三阶段续跑 A/B/C | 已做 | `empty_completion` multi-phase |
| agent tools 强制非流式再 SSE | 已做 | `proxy` |
| failover 剥 previous_response_id | 已做 | `session_affinity` |
| Claude SSE message_stop 补齐 | 已有 | `anthropic/stream` |
| 媒体 artifacts 本地路径 | 已有 | `media_artifacts` |

---

## 3. 优化方案总览（按优先级）

### 图例

- **Owner**：主要落点模块  
- **客户端收益**：C=Codex, A=Claude/Anthropic, W=WorkBuddy  
- **工作量**：S ≤1d · M 2–4d · L ≥1w  

---

### P0 — 止血（建议本周）

#### O-01 仿冒 Build 下受控注入 Codex 高频 tools

| | |
|---|---|
| **问题** | Build 面 `sanitize_responses_request_ex(..., preserve=true)` **不注入** x_search / image_gen；Codex 又常不把 MCP 放进本轮 tools → 能力真空 |
| **方案** | 在 `PlaneDecision` 增加 `inject_codex_compat_tools: bool`：`experimental && !native` 为 true；sanitize 时允许注入精简集：`x_search`、`image_gen`（可选 `web_search`）。Native TUI 仍 false |
| **Owner** | `sanitize.rs` + `build_plane_route` + `proxy` |
| **收益** | C 高 |
| **工作量** | S–M |
| **验收** | C-01/C-02：新会话「搜 X」首轮可直调，无需 curl MCP |

#### O-02 简易 Tools HTTP API（旁路标准入口）

| | |
|---|---|
| **问题** | tools 未注入时 curl JSON-RPC 运维感极强（Codex/WorkBuddy 均痛） |
| **方案** | 新增同步封装：`POST /v1/tools/{name}` body=args → `{ok, result|error, artifacts?}`；内部调同一 MCP 实现。鉴权同 local token |
| **Owner** | `server.rs` + MCP catalog 复用 |
| **收益** | C / W 高 |
| **工作量** | M |
| **验收** | `curl .../v1/tools/x_search -d '{"query":"..."}'` 一次出结果 |

#### O-03 agents-guide 改决策树 + 旁路模板

| | |
|---|---|
| **问题** | 绝对禁止降级 vs 工具不可见 → 模型夹在合规与交差之间 |
| **方案** | 改写 `integrations` 生成的 `agents-guide.md`：① 已注入 → 直调 ② health 通未注入 → **只允许** `/v1/tools/*` 或固定 MCP JSON-RPC 模板 ③ health 挂 → 声明降级。图片默认 GrokGo 媒体路径优先于「不稳的内置 imagegen」 |
| **Owner** | `integrations.rs` agents_guide 模板 |
| **收益** | C / W 高 |
| **工作量** | S |
| **验收** | 无 tools 时模型能按模板旁路，不再先 web_search |

#### O-04 分层错误码 + 友好文案

| | |
|---|---|
| **问题** | `aborted` / ECONNREFUSED / unsupported 无法区分网关挂、代理误伤、上游超时、工具假超时 |
| **方案** | 统一 `ProxyError` 枚举：`GATEWAY_DOWN` / `UPSTREAM_TIMEOUT` / `TOOL_TIMEOUT` / `TOOL_FAILED` / `ACCOUNT_COOLDOWN` / `CANCELLED`；JSON `error.type` + `retryable` + `hint`；Anthropic 路径映射到 `api_error`/`invalid_request_error` 且 message 含 stage |
| **Owner** | `proxy.rs` / `error.rs` / anthropic error body |
| **收益** | C / A / W |
| **工作量** | M |
| **验收** | 停网关时客户端看到「本地网关未启动」类 hint，不是裸 502 |

#### O-05 视频/长 MCP 超时语义：超时 ≠ 失败

| | |
|---|---|
| **问题** | `video_generate` MCP -32001 超时，产物已在 `artifacts/`，模型当失败 |
| **方案** | ① MCP 长工具：超时返回 `{ok:false, code:TOOL_TIMEOUT, retryable:true, job_id?, artifacts_hint}` ② 超时后扫 job_affinity / artifacts 目录匹配最近 job ③ 文案明确「等待超时，任务可能仍在跑」 |
| **Owner** | MCP tool handlers + `job_affinity` + `media_artifacts` |
| **收益** | W 高，C 中 |
| **工作量** | M |
| **验收** | 超时响应含可 poll 的 id 或已发现的 artifacts 路径 |

#### O-06 Claude thinking 默认可隐藏

| | |
|---|---|
| **问题** | reasoning → thinking 块拖 TTFT（D-003） |
| **方案** | config `anthropicThinkingMode: passthrough \| hide \| summary`，默认 **hide**；hide 时不向客户端发 thinking block，仍可打 debug 日志 |
| **Owner** | `anthropic/response.rs` + `stream.rs` + Settings 一行开关 |
| **收益** | A 高 |
| **工作量** | S |
| **验收** | hide 下 stream 首包为 text/tool，短问体感加速 |

---

### P1 — 体验加固（1–2 周）

#### O-07 长任务统一 submit / poll

| | |
|---|---|
| **方案** | 视频（及可选 image quality）统一：`POST /v1/jobs` 或现有 `/videos/generations` 返回 `request_id`；MCP 只 submit + 返回 poll 指引；可选 `GET /v1/jobs/{id}` 聚合状态。与 O-05 衔接 |
| **Owner** | `proxy` media + MCP |
| **收益** | W / C |
| **工作量** | L |

#### O-08 请求级可观测（响应头 + 日志）

| | |
|---|---|
| **方案** | 响应头（可选，debug 或始终）：`x-grokgo-plane`、`x-grokgo-account`（短 hash）、`x-grokgo-upstream-ms`、`x-grokgo-truncated`、`x-grokgo-thinking-mode`；日志固定三元组 `configured/routed/upstream` model |
| **Owner** | `proxy` / anthropic / usage log |
| **收益** | 全端排障 |
| **工作量** | S–M |

#### O-09 上下文裁剪可观测 + 最近优先

| | |
|---|---|
| **方案** | Claude 路径裁剪时打 `truncated_count` / before/after tokens；优先保留最近 N 轮 tool_result；header `x-grokgo-truncated: 1` |
| **Owner** | `payload_optimize` + anthropic proxy |
| **收益** | A |
| **工作量** | M |

#### O-10 模型分档映射（haiku/sonnet/opus）

| | |
|---|---|
| **方案** | 集成写入时 haiku→更快 non-reasoning，sonnet→4.5 默认，opus→4.5-build/重 reasoning；UI 显示壳→真模型 |
| **Owner** | `integrations` + `map_client_model` |
| **收益** | A / W |
| **工作量** | S |

#### O-11 MCP 工具结果 envelope 标准化

| | |
|---|---|
| **方案** | tools/call 与 `/v1/tools/*` 统一：`{ok, tool, summary, artifacts[], error?:{code,retryable,message}, raw?}` |
| **Owner** | MCP 出站层 |
| **收益** | W / C |
| **工作量** | M |

#### O-12 图像入站预检

| | |
|---|---|
| **方案** | 解码前检查宽高 ≥ 上游下限（如 8 或 512 像素积），否则 Anthropic/OpenAI 形错误 + 可操作 hint |
| **Owner** | anthropic request / proxy vision 入口 |
| **收益** | A / C |
| **工作量** | S |

#### O-13 integer 参数宽松 coerce（网关侧 MCP）

| | |
|---|---|
| **问题** | `session_id: 60619.0` 被拒（Codex 侧为主，网关 MCP 二次校验也会炸） |
| **方案** | MCP args 解析：JSON number 若 `fract()==0` 且目标 schema integer → 转 i64 |
| **Owner** | MCP 参数校验 |
| **收益** | C |
| **工作量** | S |

---

### P2 — 语义与架构（按需）

| ID | 项 | 说明 | 工作量 |
|---|---|---|---|
| O-14 | UI plane 可视化 | Settings/日志页：chat plane / media plane / experimental 开关说明一行 | S |
| O-15 | count_tokens 标注 estimate | 响应加 `x-grokgo-token-count-mode: estimate`；docs | S |
| O-16 | cache_control 诚实化 | 剥离或 warning header `upstream-prefix-only` | S |
| O-17 | 未知 content block 日志/400 | document 等高风险勿静默丢 | S |
| O-18 | 健康检查增强 | `GET /health` 增加 `accountsRoutable`、可选 upstream probe | S |
| O-19 | 集成页策略矩阵 | Native vs Experimental tools/empty-completion 表（对接 Codex 报告 §7） | S |
| O-20 | Messages→Responses 专线评估 | 仅设计文档 + spike，不默认开工 | L |

**不在 GrokGo 内做（边界）：**

- WorkBuddy system prompt 瘦身 / ToolSearch 产品行为  
- Codex `tool_search_always_defer_mcp_tools` 产品默认（可文档建议）  
- conversation_search 与 projects 索引（WorkBuddy 侧）  
- 网关进程自动拉起（可用 launchd，非核心代码）

---

## 4. 建议实施顺序（可执行里程碑）

### Milestone 1 — 工具面回填（最大痛点）

1. O-01 受控注入 Codex tools  
2. O-02 `/v1/tools/{name}`  
3. O-03 agents-guide 决策树 + 旁路模板  
4. O-13 int coerce  

**验收：** Codex C-01/C-02；旁路一条 curl 完成 x_search。

### Milestone 2 — 错误与长任务

5. O-04 分层错误码  
6. O-05 视频超时语义 + artifacts 回收  
7. O-07 submit/poll（可拆二期）  
8. O-18 health 增强  

**验收：** 视频超时不引导模型「当彻底失败」；网关挂有可读错误。

### Milestone 3 — Claude 体感

9. O-06 thinking hide 默认  
10. O-09 裁剪可观测  
11. O-10 模型分档  
12. O-12 图像预检  

**验收：** D-003/D-004/D-010 文档用例通过。

### Milestone 4 — 观测与契约

13. O-08 响应头埋点  
14. O-11 envelope  
15. O-14/O-19 UI + 矩阵文档  

---

## 5. 策略矩阵（目标态，建议写进代码注释 + Integrations）

| 客户端 | 聊天上游 | 注入 Codex tools | empty-completion | Files offload | 媒体 |
|---|---|---|---|---|---|
| Native Grok Build TUI | cli-chat-proxy | 否 | 否 | 否 | console |
| Codex 仿冒 Build | cli-chat-proxy | **是（O-01）** | 是（A/B/C） | 否 | console + 本地 artifacts |
| Codex console | api.x.ai | 是 | 是 | 可 | console |
| Claude Code | cli-chat-proxy（仿冒时） | N/A（自有 tools） | 可选 | 否 | vision 走 chat |
| WorkBuddy Chat | cli-chat-proxy（仿冒时） | N/A | 有限 | 否 | MCP /v1/tools |

---

## 6. 风险与约束

1. **O-01 注入 tools 与 Build 缓存**：只对 experimental 开；注入后 body 变化可能影响 prefix cache——应用稳定 tool 列表顺序、避免每轮变 schema。  
2. **O-05/O-07 异步**：需兼容已有 `/videos/{id}` job_affinity，勿拆两套 job 存储。  
3. **错误码变更**：保持 OpenAI/Anthropic 外壳字段，扩展放 `error.param` / `error.code` / header，避免客户端硬解析挂掉。  
4. **thinking hide**：可能影响「要看推理」的用户 → 必须可配置。

---

## 7. 不建议优先做的

| 项 | 原因 |
|---|---|
| 仿冒面全量模拟 Grok Build TUI 工具生态 | 投入巨大，产品边界错误 |
| 默认打开 SSE 全量 buffer | 与 TTFT 目标冲突；已有 tools 强制非流式 |
| 在网关实现 TodoGate | 无 Codex todo 状态 |
| 解决系统代理误伤 8787 | 属本机环境/客户端 `NO_PROXY`，可文档化 |

---

## 8. 建议的首周交付切片（若立刻开工）

按 **ROI × 三端覆盖**：

1. **O-01** 仿冒面注入 x_search + image_gen  
2. **O-03** agents-guide 决策树  
3. **O-06** Claude thinking hide  
4. **O-04** 最小错误码（GATEWAY_DOWN / UPSTREAM_TIMEOUT）  
5. **O-05** 视频 MCP 超时文案 + artifacts 探测  

预计 **2–4 人日**可形成一轮可感知体感提升；O-02/O-07 放第二周。

---

## 9. 文档与回归

新增/更新：

- 本方案：`docs/experimental-build-optimization-plan.md`（本文）  
- 对称 Codex：`docs/codex-compat-defects.md`（从 Codex 报告提炼 ID，可选）  
- `llm-wiki`：策略矩阵 + log 条目  
- Live 回归（`GROK_GO_LIVE=1`）：C-01 tools 注入、CC tool 50 次、视频 timeout 语义  

---

## 10. 一句话

三端都说「能用」；真正要靠 **GrokGo 代码**修的，是：

> **仿冒 Build 补回工具面（注入 + 简易 Tools API + 决策树）→ 长任务假失败（超时语义/异步）→ 错误与观测可读 → Claude thinking/裁剪体验。**

先出方案；确认优先级后按 Milestone 1 开工即可。
