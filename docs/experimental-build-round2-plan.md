# 仿冒 Grok Build · 第二轮优化计划（三端验收后）

> 日期：2026-07-16  
> 输入：  
> 1. Codex 验收 `Agent 实验室/grok-4.5-仿冒-build-修复验收-2026-07-16.md`  
> 2. Claude Code 复测 `docs/claude-code-compat-fix-review.md`  
> 3. WorkBuddy 复验 `WorkBuddy/.../grok-build-compat-reverify.md`  
> 对照：`docs/experimental-build-optimization-plan.md`（O-01–O-19 已落地）  
> 原则：**只改 GrokGo 可交付部分；任何改动必须标明端别影响，默认不伤主路径**

---

## 1. 三端共识

### 1.1 本轮已经「修对」的

| 能力 | Codex | Claude Code | WorkBuddy |
|---|---|---|---|
| experimental-build 可跑 | ✅ 注入 + plane 头 | ✅ Messages 主环 | ✅ chat + tool_calls 更稳 |
| 工具入口 | ✅ `/v1/tools/*` | N/A（自有 tools） | ✅ schema 更完整 |
| 长任务假失败 | 部分（video 路径） | N/A | ✅ `wait=false` + poll **最大赢点** |
| 可观测 | plane/account/ms | 头齐全 + thinking hide | health + envelope 壳 |
| thinking 可见首包 | 间接 | ✅ hide 默认 | N/A |

### 1.2 共识残留（真正还差）

```text
网关能力升级了
    ↓
运行时指令 / 宿主配置 / 结果形态 没完全跟上
    ↓
模型仍绕路、仍二次解析、仍可能踩默认同步超时
```

| 共性痛点 | Codex | Claude | WorkBuddy | 是否 GrokGo 可改 |
|---|---|---|---|---|
| **结果可消费性** | x_search 埋 `raw` | — | 同，壳有肉无 | **是** |
| **运行时 guide/配置未刷新** | 磁盘 agents-guide 旧 | CC env 仍全 `grok-4.5` | 宿主两跳/肥上下文 | 部分（注入刷新） |
| **长任务默认仍同步** | MCP/旁路 | — | wait 默认 true | **是**（要控影响面） |
| **模型身份字段拧** | 有 plane 头 | 有 | 请求/返回/models 分裂 | **是**（要兼容） |
| 绝对时延/抖动 | 仍有 | D-005 未破 | 略好 | 有限（上游为主） |
| 宿主架构债 | Codex tool 列表 | CC effort/env | ToolSearch/conversation | **否 / 文档** |

### 1.3 产品边界（本轮不破）

```text
GrokGo = SuperGrok 会话面 + 本地工具/媒体网关 + 各协议壳
≠ 改 Codex / Claude Code / WorkBuddy 上游产品
≠ 100% Grok Build TUI 同构
≠ 解决上游绝对时延
```

---

## 2. 跨端安全护栏（所有改动必须过）

任何优化在合入前用下表自检：

| 护栏 | 说明 |
|---|---|
| **G-1 Native Build TUI 不伤** | 不注入 Codex tools；不默认 empty-completion；不改官方 tool 列表顺序 |
| **G-2 默认同步契约不静默变语义** | 破坏性默认（如 MCP `wait=false`）仅对 **MCP tools/call** 或显式 agent 路径生效；`POST /v1/tools/*` 与 CLI 同步调用默认保持 `wait=true`（或 `wait` 缺省兼容旧行为） |
| **G-3 结果形态向后兼容** | envelope 可 **增** 字段（`result`/`data`/`text`），不删 `ok`/`path`/`markdown`；`raw` 默认可省略但 `?raw=1` / `debug:true` 可还原 |
| **G-4 OpenAI/Anthropic 外壳字段** | `error.message` / `error.type` 仍可被旧客户端解析；扩展放 `code`/`retryable`/`hint`/header |
| **G-5 响应 `model` 字段** | 客户端若只读 `model`，不要突然改成未知 id 导致 UI 崩；**新增** `requested/routed/upstream` 或响应头，列表 alias 可加不可删 |
| **G-6 agents-guide 刷新** | 仅覆盖 GrokGo 托管块；version bump 强制写；不删用户自写 AGENTS 其它段落 |
| **G-7 Claude thinking** | 默认保持 `hide`；passthrough 必须仍可用 |
| **G-8 多账号** | 任何 poll/job 继续走既有 `job_affinity`，禁止第二套 job 存储 |

---

## 3. 第二轮工作项（R2）

图例：工作量 S ≤1d · M 2–4d · L ≥1w；收益端 C=Codex A=Claude W=WorkBuddy

---

### P0 — 交付闭环（建议本周，跨端高 ROI）

#### R2-01 强制刷新运行时 agents-guide

| | |
|---|---|
| **问题** | 源码模板已是决策树，但 `~/.grok-go/agents-guide.md` 仍是 0.1.8 旧文 → Codex 体感几乎不涨 |
| **方案** | ① 网关 **start** 与 Integrations 注入时调用 `ensure_agents_guide_file` ② 文件头 version ≠ `CARGO_PKG_VERSION` 则覆盖 ③ UI「刷新工具指引」按钮（若无则 Settings/集成一行） |
| **Owner** | `integrations.rs` + gateway start 钩子 |
| **收益** | C 极高，W 间接 |
| **护栏** | G-6；不碰 Codex 用户自定义 AGENTS 非托管段 |
| **验收** | 启动后磁盘 guide 含「决策树」「/v1/tools」「TOOL_TIMEOUT」 |

#### R2-02 工具结果「可直接吃」envelope（x_search 优先）

| | |
|---|---|
| **问题** | 三端都说：壳有了，答案埋在 `raw`，summary 无信息量 |
| **方案** | 统一成功形态： |
| | ```json |
| | { "ok": true, "tool": "x_search", "summary": "<一句话>", |
| |   "result": { "text": "...", "citations": ["https://x.com/..."] }, |
| |   "artifacts": [], "raw": null } |
| | ``` |
| | ① 从 Responses `output` 抽 message 文本 + 链接 ② 默认 **不** 塞巨型 `raw` ③ `arguments.debug=true` 或 query `?raw=1` 才带 raw ④ image/video 成功路径保证 `artifacts[]` + 既有 `path/markdown` 并存 |
| **Owner** | `server.rs` handle_tool_call + 抽取 helper |
| **收益** | C / W 高 |
| **护栏** | G-3；MCP content 仍是 text JSON 字符串，字段只增不硬删 path/markdown |
| **验收** | `POST /v1/tools/x_search` 不挖 raw 可读；image_gen artifacts 非空 |

#### R2-03 长任务：MCP 默认异步 + 同步超时必带 job_id

| | |
|---|---|
| **问题** | WorkBuddy：`wait=false` 已通，但默认 `wait=true` 仍可能 MCP -32001 |
| **方案** | ① **仅** MCP `tools/call` 路径：`video_generate` 缺省 `wait=false`（或 config `mcp_video_wait_default=false`） ② `POST /v1/tools/video_generate` **保持** 缺省 `wait=true`（人类/curl 同步） ③ 无论同步超时：响应必须含 `error.code=TOOL_TIMEOUT`、`retryable=true`、`job_id`、`poll`、`artifacts` 探测 ④ agents-guide：Agent 场景写死「视频默认异步」 |
| **Owner** | `server.rs` + config 可选 + guide |
| **收益** | W 高，C 中 |
| **护栏** | **G-2 关键**：禁止全局默认改成 false 误伤 Tools HTTP/同步脚本 |
| **验收** | MCP 无 wait → 立刻 job_id；Tools HTTP 无 wait → 仍同步等到结果或显式超时带 job_id |

#### R2-04 集成注入「自检生效」

| | |
|---|---|
| **问题** | Claude：网关 O-10 已好，本机 env 仍三档 `grok-4.5`；Codex guide 未刷 |
| **方案** | ① CC Switch / Claude 注入后读回 env，断言 HAIKU≠全 4.5（或写入 claude-haiku-* 壳名） ② 注入完成后返回 `IntegrationStatus` 含 `agentsGuideVersion` / `claudeHaikuModel` ③ 可选：启动时若检测到旧 haiku=`grok-4.5` 且开启 auto-inject，log warn |
| **Owner** | `integrations.rs` + UI 状态一行 |
| **收益** | A 高，C 中 |
| **护栏** | 不强制覆盖用户手改的 `ANTHROPIC_MODEL` 除非用户点「重新注入」 |
| **验收** | 一键注入后 haiku 为 `claude-haiku-4-5` 或 non-reasoning；guide version 对齐 |

---

### P1 — 体感加固（1–2 周）

#### R2-05 模型身份三联 + models 列表诚实化

| | |
|---|---|
| **方案** | 响应 JSON 增加（OpenAI/Chat 兼容扩展字段，不替换 `model`）：`requested_model` / `routed_model` / `upstream_model`（或 header `x-grokgo-model-requested|routed|upstream`） ② `/v1/models` 增加 `grok-4.5-build` **alias**（指向同一能力）或文档说明 build 面重写规则 |
| **收益** | W / C / A 排障 |
| **护栏** | G-5：`model` 主字段策略固定写进注释（推荐：客户端请求名 **或** 稳定对外 id，二选一并文档化，避免每轮变） |

#### R2-06 响应头 `x-grokgo-tools-injected`

| | |
|---|---|
| **方案** | experimental inject 时：`x-grokgo-tools-injected: x_search,image_gen`；native：省略或 `none` |
| **收益** | C 排障 |
| **护栏** | 仅 header，不改 body |

#### R2-07 agents-guide 图片语义去打架

| | |
|---|---|
| **方案** | 统一：仿冒/Codex 路径 **优先 GrokGo `image_gen`（已注入或 MCP）**；仅当会话已有更稳原生 image 工具时用原生。删除「默认 Codex 内置 imagegen」与 Branch A 冲突句 |
| **收益** | C |
| **护栏** | G-6 |

#### R2-08 Claude 多轮时延可拆分埋点（不承诺变快）

| | |
|---|---|
| **方案** | 日志/可选 header：`convert_ms` / `optimize_ms` / `upstream_ttfb`（有则） ② 文档 KPI：同账号 20 次短问/tool 基线 ③ **不**默认 SSE 全量 buffer |
| **收益** | A |
| **护栏** | 不打开会伤 TTFT 的全局 buffer |

#### R2-09 poll 成功时本地 artifacts 回收

| | |
|---|---|
| **方案** | `GET /v1/videos/{id}` 在 `done` 时尽量 materialize 并返回 `artifacts[]`/`path`（与 MCP 成功同形） |
| **收益** | W / C |
| **护栏** | G-8 沿用 job_affinity |

#### R2-10 MCP 失败统一 `ok=false + code + retryable`

| | |
|---|---|
| **方案** | tools/call 错误路径一律 envelope（含 transport 分类）；与 R2-02 成功形态对称 |
| **收益** | 全端 |
| **护栏** | G-4；MCP `isError` 与 envelope.ok 一致 |

---

### P2 — 按需 / 宿主边界

| ID | 项 | 说明 | GrokGo? |
|---|---|---|---|
| R2-11 | Settings 文案 | thinking hide ≠ 上游不思考（半句） | 是 |
| R2-12 | D-002 回归套件 | 多段 tool_result / 并行 tool_use 单测 | 是 |
| R2-13 | 未知 content block 计数 | 日志 + 可选 metric，勿默认 400 扩大面 | 是 |
| R2-14 | Codex C-01 新会话验收 | 「搜 X cgnot996」零旁路 | 流程 |
| R2-15 | WorkBuddy ToolSearch 常驻 | 宿主配置建议文档 | **否**（文档） |
| R2-16 | conversation_search | 宿主 | **否** |
| R2-17 | 系统上下文瘦身 | 宿主 | **否** |
| R2-18 | Codex write_stdin float | 客户端 schema | **否**（文档边界） |
| R2-19 | O-20 Messages→Responses | spike only | 评估 |
| R2-20 | 网关单点 / launchd | 运维 | 文档可选 |

---

## 4. 实施顺序（里程碑）

### Milestone A — 交付闭环（约 2–3 人日）

1. R2-01 guide 强制刷新  
2. R2-02 x_search/result 可消费 + artifacts 统一  
3. R2-03 MCP 视频默认异步（config 开关，Tools HTTP 默认同步）  
4. R2-04 注入自检  

**验收：** 新 Codex 会话读到新 guide；curl tools x_search 可读；MCP video 无 wait 返回 job_id；Claude 重注入后 haiku 分档。

### Milestone B — 排障与 poll 体验

5. R2-05 模型身份三联/alias  
6. R2-06 tools-injected 头  
7. R2-07 guide 图片语义  
8. R2-09 poll 带 artifacts  
9. R2-10 MCP 错误 envelope  

### Milestone C — 观测与回归

10. R2-08 时延拆分  
11. R2-11–R2-13 文案/套件  
12. R2-14 实机 C-01  

---

## 5. 策略矩阵（第二轮目标态）

| 路径 | wait 默认 | 结果形态 | guide/注入 |
|---|---|---|---|
| Native Grok Build TUI | n/a | 上游原样 | 不注入 tools |
| Codex Responses 仿冒 | n/a（server tools） | inject x_search/image_gen | 磁盘 guide 强制新决策树 |
| `POST /v1/tools/*` | **同步 wait=true** | compact result，raw 可选 | curl 模板在 guide |
| MCP `tools/call` 视频 | **async wait=false** | job_id+poll；完成态 artifacts | guide 写死 Agent 异步 |
| Claude Messages | n/a | thinking hide；分档映射 | 注入 haiku 壳名/non-reasoning |
| WorkBuddy Chat | 跟 MCP | 同 MCP envelope | 宿主两跳文档建议 |

---

## 6. 风险与「别踩」

1. **MCP 默认 wait=false**：若错误地改到 Tools HTTP，会让「curl 一次出片」脚本变异步——必须路径分流（R2-03）。  
2. **丢掉 raw**：调试能力下降 → 必须 debug 开关。  
3. **强制 tool_choice 指向 x_search**：可能干扰 Codex 本机 shell 工作流 → 本轮不做，仅靠描述/结果折叠（可选后续）。  
4. **重写 `model` 为 grok-4.5-build**：WorkBuddy/账单困惑 → 用扩展字段/ alias，不突然换主字段语义。  
5. **guide 每次启动重写**：确保仅托管文件且 version 比较，避免无意义 IO。

---

## 7. 不建议本轮做的

| 项 | 原因 |
|---|---|
| 默认 SSE 全量 buffer | 与 Claude/Codex TTFT 目标冲突 |
| 全量模拟 Build TUI 工具 | 边界错误 |
| 改 WorkBuddy ToolSearch 产品默认 | 非本仓 |
| 修 Codex write_stdin float | 客户端校验 |
| 承诺绝对时延 P50 指标作为发布门禁 | 上游主导；只做可观测 |

---

## 8. 一句话

> **第一轮补了网关能力；第二轮要把能力「交到模型手里」：刷新运行时指令、结果可直接吃、MCP 长任务默认异步且不伤同步 HTTP，并让 Claude 注入真正吃到分档——全程用路径分流护栏保证三端互不踩踏。**

确认优先级后建议按 **Milestone A（R2-01 → R2-04）** 开工。
