# Claude Code × GrokGo 兼容性缺陷清单

> 状态：实测 + 代码审阅  
> 日期：2026-07-16  
> 范围：`ANTHROPIC_BASE_URL → GrokGo :8787 → Grok Build / xAI Chat` 路径  
> 客户端：Claude Code 2.1.185  
> 后端模型：`grok-4.5` / `grok-4.5-build`（仿冒 Grok Build 渠道）

---

## 1. 结论摘要

当前链路**主路径可用**（文本对话、tool_use 闭环、SSE 流式、模型列表），但属于：

```text
Claude Code (Anthropic Messages)
  → GrokGo Anthropic 兼容层
  → OpenAI Chat Completions 形态
  → Grok Build / xAI 上游
  → 再映射回 Anthropic Messages / SSE
```

这是**协议兼容**，不是**语义等价**。

| 维度 | 评分 | 说明 |
|---|---:|---|
| 协议打通 | 8/10 | `/v1/messages`、tools、stream 基本可用 |
| 长会话稳定 | 6/10 | 靠上下文裁剪保活，会丢细节 |
| 缓存语义 | 4/10 | 有 `cache_read` 字段，无 Claude 级 `cache_control` 语义 |
| 延迟体验 | 5/10 | thinking + Build 上游 + 本机代理叠加 |
| 边缘能力 | 4/10 | 图像规则、count_tokens、beta 能力偏弱 |
| 整体可日常用 | 7/10 | 能干活；不适合当原生 Claude 替代品做关键深改 |

**一句话：**  
能跑 Claude Code agent 主循环，但延迟、上下文可控性、缓存与计量、多模态边界都明显弱于官方 Claude。

---

## 2. 实测基线（本机 2026-07-16）

环境：

- `ANTHROPIC_BASE_URL=http://127.0.0.1:8787`
- 本地网关进程：`grok-go` 监听 `127.0.0.1:8787`
- 默认模型映射：`opus → grok-4.5`
- `CLAUDE_EFFORT=high`

| 用例 | 结果 | 延迟 | 备注 |
|---|---|---:|---|
| 最小文本 `ping→pong` | 通过 | ~1.1s | 响应含 `thinking` |
| 中文短问 | 通过 | ~4.4s | 先 thinking 再 text |
| stream 首包 | 通过 | ~0.4s | 首包常是 thinking_delta |
| tools 触发 `get_time` | 通过 | ~1.5s | `stop_reason=tool_use` 正确 |
| tool_result 回传 | 通过 | ~12.8s | 多轮抖动明显 |
| `cache_control` system 块 | 可接收 | ~3.3s | 不代表官方 cache 语义 |
| 1×1 PNG image | 上游 400 | ~0.8s | 尺寸限制，非协议未识别 |
| `/v1/messages/count_tokens` | 有响应 | ~1ms | 本地启发式估算 |
| 直连 `api.x.ai` | 超时 | — | 依赖代理/账号池链路 |

---

## 3. 缺陷清单（按修复优先级）

优先级定义：

- **P0**：会直接导致工具循环失败、会话中断、错误行为
- **P1**：显著影响日常编码效率或答案质量
- **P2**：边界能力缺失 / 体验瑕疵
- **P3**：观测性、文档、长期架构债

---

### P0 — 工具循环正确性

#### D-001 流式 tool_use 生命周期必须严格

**现象**  
Claude Code 强依赖 SSE 顺序：

1. `message_start`
2. `content_block_start`（tool_use 必须带 `id` + `name`）
3. `input_json_delta` 分片
4. `content_block_stop`
5. `message_delta.stop_reason=tool_use`
6. `message_stop`

任一环节缺 `id`、`stop_reason` 错、中途断流未补 `message_stop`，都会导致工具不执行或会话卡死。

**现状**  
代码已有针对性处理，但属于高回归区。

**代码点**

- `src-tauri/src/gateway/anthropic/stream.rs`
  - `OpenAiToAnthropicSse`
  - `finish()` 在上游中断时合成 `message_stop`
- `src-tauri/src/gateway/anthropic/response.rs`
  - `map_stop_reason()`：即使 `finish_reason=stop`，只要有 tool_calls 也强制 `tool_use`

**建议**

1. 增加 live 回归：并行多 tool、arguments 超长、中途 abort、空 arguments
2. 对缺 `id` 的 tool_call 合成稳定 id，并打 error 日志
3. 禁止在 content 含 tool_use 时输出 `stop_reason=end_turn`

**验收**

- 连续 50 次 “强制调工具” 无卡死
- abort 场景下客户端不报 mid-response connection closed 后无法恢复

---

#### D-002 tool_result / 多内容块拆分边界

**现象**  
Anthropic 允许同一 user message 内混排 `text + tool_result`；OpenAI Chat 需要拆成 `role=tool` 独立消息。拆错会导致：

- 上游 4xx
- 模型看不到 tool 输出
- 历史回放错位

**代码点**

- `src-tauri/src/gateway/anthropic/request.rs`
  - `convert_message()`
  - `flatten_tool_result_content()`
  - tool_use / tool_result 映射

**建议**

1. 固定规则：同一 user 中先输出全部 `tool` 消息，再输出剩余 text；或严格按出现顺序拆
2. `tool_result.content` 数组块（text/image）统一 flatten 策略写清
3. 对 `is_error=true` 显式注入错误前缀，避免模型当正常结果

**验收**

- 单轮多 tool_result
- text+tool_result 混排
- 大 tool_result（>32KB）回传后仍能继续

---

### P1 — 日常体验与质量

#### D-003 thinking 透出导致 TTFT/体感延迟变差

**现象**  
上游 reasoning 被映射为 Anthropic `thinking` 块。  
Claude Code 会先消费 thinking，再显示最终文本，短回复也显得慢。

**实测**

- `ping` 非流式约 1.1s，仍先 thinking 再 `pong`
- stream 首包常是 thinking_delta，不是 text_delta

**代码点**

- `src-tauri/src/gateway/anthropic/response.rs`  
  `reasoning_content` / `reasoning` → `type=thinking`
- `src-tauri/src/gateway/anthropic/stream.rs`  
  thinking block start/delta
- 请求侧会剥离历史 thinking：  
  `request.rs` 中 `"thinking" | "redacted_thinking" => drop`

**建议（可选策略，建议做成配置）**

| 模式 | 行为 | 适用 |
|---|---|---|
| `thinking=passthrough` | 现状，透出 reasoning | 调试 / 研究 |
| `thinking=hide`（推荐默认） | 不向 Claude Code 发 thinking 块 | 日常编码提速 |
| `thinking=summary` | 压缩为短摘要或日志 | 需要可观测但不拖 UI |

**验收**

- hide 模式下短回复可见文本时间显著下降
- 不影响 tool_use 触发率

---

#### D-004 长上下文静默裁剪导致“假失忆”

**现象**  
为避免上游长 SSE 断流，Claude 路径会做 token-aware 裁剪：

- soft budget：约 80k estimated tokens
- hard budget：约 100k
- 更早的 tool/user 大文本 head+tail stub
- 必要时压缩 tools schema

好处：保活。  
代价：模型像忘了早期文件内容 / 工具输出，且客户端未必知情。

**代码点**

- `src-tauri/src/gateway/payload_optimize.rs`
  - `CHAT_TOKEN_SOFT = 80_000`
  - `CHAT_TOKEN_HARD = 100_000`
  - `CHAT_CONTEXT_WINDOW = 128_000`
  - `enforce_chat_context_budget()`
  - `truncate_message_content()` / historical stub

**建议**

1. 裁剪时在 response headers 或本地日志输出：
   - `x-grokgo-context-tokens-before/after`
   - `x-grokgo-truncated-count`
2. 对“最近 N 轮 tool_result / 当前编辑文件相关内容”提高保留优先级
3. 大 blob 优先 Files offload，而不是直接截断成不可检索 stub
4. 在 Claude Code 集成说明中明确：长会话建议分段

**验收**

- 100+ tool 轮后仍能引用最近 10 轮关键结果
- 触发裁剪时用户/日志可感知

---

#### D-005 count_tokens 为启发式估算，不可用于精确预检

**现象**  
`POST /v1/messages/count_tokens` 近乎瞬时返回，实现是：

```text
chars / 4 + 8
```

Claude Code 若据此做上下文预检/截断决策，会系统性偏差。

**代码点**

- `src-tauri/src/gateway/anthropic/request.rs`
  - `estimate_token_count()`
- `src-tauri/src/gateway/server.rs`
  - `/v1/messages/count_tokens` 路由

**建议**

1. 文档明确标注 “estimate only”
2. 中期：按模型族维护更接近 xAI 的计数器，或透传上游 tokenizer（若有）
3. 与 `payload_optimize` 共用同一估算器，避免两套数

**验收**

- 估算误差范围可文档化（例如 ±20% 目标）
- 不因 count_tokens 误判导致过早/过晚裁剪

---

#### D-006 cache_control 无 Claude 语义，usage 易误导

**现象**

- 请求中的 `cache_control` 不会按 Anthropic 语义生效
- 响应可能出现 `cache_read_input_tokens`
- 该值来自上游 cached_tokens 映射，不等于 Claude ephemeral cache breakpoint

**代码点**

- 请求：`request.rs` 明确 strip Anthropic-only 字段（含 cache/thinking/metadata）
- 响应：`response.rs` 从  
  `usage.prompt_tokens_details.cached_tokens`  
  映射到 `cache_read_input_tokens`
- 会话黏连：`src-tauri/src/session_affinity.rs`  
  `prompt_cache_key` / 账号亲和

**建议**

1. 文档写清：当前是“上游前缀缓存/会话黏连”，不是 Claude prompt caching API
2. 若无法实现 breakpoint cache，考虑：
   - 忽略并剥离 `cache_control`
   - 或返回 warning header：`x-grokgo-cache-mode: upstream-prefix-only`
3. 避免在 UI/日志把 cache_read 宣传成 Claude 官方 cache hit

**验收**

- 用户不再按官方 cache 计费/命中率做决策
- 账号切换后 cache 失效有可观测信号

---

#### D-007 多轮 tool 时延抖动大

**现象**  
单次 tool_use 尚可，tool_result 回传后下一跳延迟可到 10s+。  
对 Claude Code 这种“读→改→再读”密集工具流，体感放大明显。

**根因组合**

1. Build 上游本身慢/thinking 重
2. 本机多一跳 `127.0.0.1:8787`
3. 账号池/会话维护
4. 上下文随轮次膨胀后的 optimize 开销

**代码点**

- 网关总路径：`gateway/server.rs` → `proxy_anthropic_messages`
- 上下文优化：`payload_optimize.rs`
- Build 会话维护：`integrations.rs` / auth 刷新逻辑

**建议**

1. 增加 per-stage 耗时埋点：
   - convert_ms
   - optimize_ms
   - upstream_ttfb_ms
   - upstream_total_ms
   - map_ms
2. 对 Claude 路径默认降低 reasoning 强度（若上游支持）
3. 热路径避免重复 JSON 深拷贝/多次全量估算

**验收**

- p50 tool 回传后下一跳 < 3s（本机空载）
- p95 有日志可定位是 convert / optimize / upstream

---

### P2 — 边界能力与协议覆盖

#### D-008 图像输入受上游规则限制，错误不够“Claude 化”

**现象**  
兼容层可转发 image block，但上游对尺寸/格式更严。  
最小 1×1 PNG 返回：

```text
invalid-argument: Image dimensions 1x1 are too small...
```

Claude Code 截图、小图标、异常 media 更容易踩坑。

**代码点**

- 请求 content 转换：`request.rs` image block 映射
- 媒体相关：`gateway/image_bridge.rs`、`files_api.rs`、`media_artifacts.rs`

**建议**

1. 入站预检：宽高 < 8、空 data、超大 base64 直接转清晰 Anthropic error
2. 对常见截图自动压缩/转码策略（可配置）
3. 错误信息附“可操作建议”（放大、转 PNG/JPEG、改用 Files）

---

#### D-009 未知 content block 被静默丢弃

**现象**  
`document`、仅含 `cache_control` 的块、未来 beta block 等，在转换时 ignore。  
客户端以为发进去了，模型实际没看到。

**代码点**

- `request.rs` `convert_message()` 默认分支：Ignore unknown blocks

**建议**

1. 统计并日志化 dropped block types
2. 对高风险类型（document / image 变体）返回 400 而不是静默丢
3. 维护支持矩阵文档

---

#### D-010 模型别名与能力预期错位

**现象**

- Claude Code 配置层是 `opus/sonnet/haiku`
- 实际映射到 `grok-4.5` / `grok-4.3` 等
- 响应 model 字段为 `grok-4.5-build`

用户会按 Claude 能力边界预期，实际是 Grok Build 行为。

**代码点**

- `request.rs` `map_client_model()`
- `integrations.rs` 写入：
  - `ANTHROPIC_MODEL`
  - `ANTHROPIC_DEFAULT_{HAIKU,SONNET,OPUS}_MODEL`

**建议**

1. 集成页明确显示“壳模型 → 真模型”
2. 不同档位不要都映射到同一重模型（haiku 应走更快 non-reasoning）
3. 在 `/v1/models` 增加兼容别名说明字段（若客户端可忽略额外字段则放 docs）

---

#### D-011 schema 清洗可能改变工具语义

**现象**  
为适配上游，会 normalize JSON Schema（如去掉 `format: uri`、null 等）。  
多数情况正确，但可能让严格 schema 工具的约束变松。

**代码点**

- `src-tauri/src/gateway/anthropic/schema.rs` `normalize_schema()`
- batch tool 过滤：`is_batch_tool()`

**建议**

1. 保留清洗前后 diff 日志（debug）
2. 对 Claude Code 内置工具白名单单独测试 Read/Edit/Bash/Glob/Grep
3. 避免过度删除 `required` / `$ref` 关���结构

---

### P3 — 架构与可维护性

#### D-012 双平面协议债：Messages ⇄ Chat，而非直打 Responses

**现状设计选择**（合理但有代价）：

- Claude 路径：Messages → Chat Completions
- Codex / Build 某些路径：Responses

Chat 路径 tool 映射成熟，但与 Build 原生会话面能力不完全同构。

**代码点**

- `gateway/anthropic/mod.rs` 设计说明
- `llm-wiki/raw/anthropic-claude-code-compat.md`
- Build 平面：`gateway/build_plane_route.rs`、config 中 `cli-chat-proxy`

**建议**

- 短期继续 Chat 兼容
- 中期评估：高价值场景（缓存、推理控制、多模态）是否值得 Messages→Responses 专线

---

#### D-013 观测性不足，问题难分层

**现象**  
用户只感知“慢/傻/忘”，无法区分：

- Claude Code 本地开销
- GrokGo 转换/裁剪
- 上游 thinking
- 账号池切换
- 网络代理

**建议埋点（最小集）**

| 指标 | 说明 |
|---|---|
| `convert_ms` | Anthropic→OpenAI |
| `optimize_ms` | 上下文裁剪 |
| `upstream_ttfb_ms` | 上游首字节 |
| `upstream_total_ms` | 上游总耗时 |
| `map_ms` | OpenAI→Anthropic |
| `account_id_hash` | 账号黏连（脱敏） |
| `tokens_in_est/out` | 估算 token |
| `truncated` | 是否裁剪 |
| `thinking_chars` | reasoning 规模 |

输出位置：本地 debug log + 可选 response header。

---

## 4. 已具备、可维持的正确能力

这些不是缺陷，属于应回归保护的基线：

1. `POST /v1/messages` 主路径可用
2. `tools` → OpenAI functions 映射可用
3. `tool_choice` 对象映射（auto/any/none/tool）可用
4. `disable_parallel_tool_use` → `parallel_tool_calls=false`
5. `stop_reason` 在 tool_calls 场景强制 `tool_use`
6. 流式 `message_stop` 在上游中断时尽力补齐
7. 同时接受 `Authorization: Bearer` 与 `x-api-key`
8. `/v1/models`、`/v1/messages/count_tokens` 有兼容入口
9. Claude Code 集成写入 `ANTHROPIC_BASE_URL`（不带 `/v1`）

相关文件：

- `src-tauri/src/gateway/anthropic/{mod,request,response,stream,schema}.rs`
- `src-tauri/src/gateway/server.rs`
- `src-tauri/src/integrations.rs`
- `CHANGELOG.md` 0.1.7 / 0.1.8 相关条目

---

## 5. 推荐修复顺序（工程排期）

### Phase A — 1~2 天，体感立刻变好

1. **D-003** thinking 默认 hide / 可配置  
2. **D-013** 最小耗时埋点  
3. **D-010** haiku/sonnet/opus 分档映射到快/中/重模型  

### Phase B — 3~5 天，稳定性

4. **D-001 / D-002** 工具流与 tool_result 回归套件  
5. **D-004** 裁剪可观测 + 最近上下文优先保留  
6. **D-007** optimize/convert 热路径性能

### Phase C — 按需

7. **D-008 / D-009** 图像与未知 block 明确错误  
8. **D-005 / D-006** tokens/cache 语义诚实化  
9. **D-012** 评估 Responses 专线

---

## 6. 给 Claude Code 用户的临时规避策略

在缺陷修复前，建议：

1. **日常降 effort**：`CLAUDE_EFFORT=medium|low`
2. **轻任务别挂最重模型**：haiku 档映射到 non-reasoning
3. **长任务分段开 session**：避免无限滚工具历史
4. **大工具输出先摘要**：不要依赖网关静默截断后仍“全记得”
5. **关键深改优先官方 Claude**：Build 渠道适合吞吐与非关键任务
6. **盯健康信号**：
   - tool_use 触发率
   - tool 回传后下一跳时延
   - 空响应 / 中断重试率
   - 是否频繁触发 context budget

---

## 7. 代码索引（快速跳转）

| 主题 | 路径 |
|---|---|
| 兼容层总览 | `src-tauri/src/gateway/anthropic/mod.rs` |
| 请求转换 | `src-tauri/src/gateway/anthropic/request.rs` |
| 响应转换 | `src-tauri/src/gateway/anthropic/response.rs` |
| SSE 转换 | `src-tauri/src/gateway/anthropic/stream.rs` |
| Schema 清洗 | `src-tauri/src/gateway/anthropic/schema.rs` |
| 上下文预算 | `src-tauri/src/gateway/payload_optimize.rs` |
| 路由入口 | `src-tauri/src/gateway/server.rs` |
| 会话黏连 / cache key | `src-tauri/src/session_affinity.rs` |
| Claude Code 注入 | `src-tauri/src/integrations.rs` |
| 设计笔记 | `llm-wiki/raw/anthropic-claude-code-compat.md` |
| 版本记录 | `CHANGELOG.md`（0.1.7 引入，0.1.8 加固） |

---

## 8. 终评

| 问题 | 判断 |
|---|---|
| 能不能继续用？ | 能，主 agent 循环已通 |
| 像不像原生 Claude？ | 不像，尤其在延迟、缓存、长上下文、边缘模态 |
| 最值得先修什么？ | thinking 可开关、工具流回归、裁剪可观测、模型分档 |
| 最大产品风险 | 用户以 Claude 预期使用，实际拿到的是“Build 渠道 + 兼容层近似实现” |

---

## 附录 A. 最小复现命令

```bash
BASE=http://127.0.0.1:8787
KEY=你的本地token

# 1) 最小文本
curl -sS "$BASE/v1/messages" \
  -H "content-type: application/json" \
  -H "x-api-key: $KEY" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model":"grok-4.5",
    "max_tokens":32,
    "messages":[{"role":"user","content":"ping, reply pong only"}]
  }'

# 2) 工具触发
curl -sS "$BASE/v1/messages" \
  -H "content-type: application/json" \
  -H "x-api-key: $KEY" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model":"grok-4.5",
    "max_tokens":200,
    "tools":[{
      "name":"get_time",
      "description":"Get time",
      "input_schema":{
        "type":"object",
        "properties":{"tz":{"type":"string"}},
        "required":["tz"]
      }
    }],
    "messages":[{"role":"user","content":"Call get_time with tz=Asia/Shanghai"}]
  }'

# 3) count_tokens
curl -sS "$BASE/v1/messages/count_tokens" \
  -H "content-type: application/json" \
  -H "x-api-key: $KEY" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model":"grok-4.5",
    "messages":[{"role":"user","content":"hello"}]
  }'
```

---

## 附录 B. 缺陷 ID 速查

| ID | 优先级 | 标题 |
|---|---|---|
| D-001 | P0 | 流式 tool_use 生命周期 |
| D-002 | P0 | tool_result / 多块拆分 |
| D-003 | P1 | thinking 透出拖慢体感 |
| D-004 | P1 | 长上下文静默裁剪 |
| D-005 | P1 | count_tokens 启发式 |
| D-006 | P1 | cache_control 语义缺失 |
| D-007 | P1 | 多轮 tool 时延抖动 |
| D-008 | P2 | 图像上游限制与错误体验 |
| D-009 | P2 | 未知 content block 静默丢弃 |
| D-010 | P2 | 模型别名与能力预期错位 |
| D-011 | P2 | schema 清洗副作用 |
| D-012 | P3 | Chat vs Responses 架构债 |
| D-013 | P3 | 分层观测性不足 |
