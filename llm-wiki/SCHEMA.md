# GrokGo 项目 LLM Wiki 维护规则

## 目的

本目录是 **GrokGo 仓库内的项目知识层**，服务后续任意 Agent / 人类接手。  
它不是对外 README，也不是共享知识库；它把源码、配置、运行时约定编译成可交叉引用的 Markdown，避免每次从零读仓库。

## 方法论

| Karpathy 方法 | 本项目落点 |
|---|---|
| raw sources | `raw/` + 仓库源码 / README / docs / `~/.grok-go/agents-guide.md` |
| persistent wiki | `wiki/` |
| schema / rules | 本文件 `SCHEMA.md` + 根目录 `AGENTS.md` 引用 |
| index + log | `wiki/index.md`、`wiki/log.md` |
| questions improve wiki | `wiki/queries/` |
| accumulated synthesis | `wiki/syntheses/` |

## 目录结构

```text
llm-wiki/
  SCHEMA.md                 维护规则（本文件）
  README.md                 人类与 Agent 入口
  raw/                      来源索引（不可变快照 / 指针）
  wiki/
    index.md                全局索引与一句话摘要
    log.md                  变更日志
    syntheses/              跨模块总判断
    modules/                代码模块地图
    concepts/               可复用概念与协议
    playbooks/              操作 SOP
    queries/                高频问答
```

## 页面类型

| 类型 | 目录 | 写什么 |
|---|---|---|
| synthesis | `wiki/syntheses/` | 产品定位、整体架构、当前判断 |
| module | `wiki/modules/` | 源码模块职责、入口文件、关键数据流 |
| concept | `wiki/concepts/` | 模型映射、sanitize、鉴权等横切概念 |
| playbook | `wiki/playbooks/` | 开发、发布、调试、接手步骤 |
| query | `wiki/queries/` | FAQ 与可复用问答 |

## 每页标准结构

```markdown
# 标题

## 结论
先给可执行结论。

## 细节
关键机制、文件路径、默认值、边界条件。

## 相关页面
- [[path/without-ext]]

## 来源
- 仓库相对路径或外部文档指针
```

## 更新规则

1. **先改 wiki 再改代码文档** 不是硬性要求；但代码行为变化后，必须回写相关 wiki 页。
2. 优先更新已有页，避免平行页面（同一概念不要开两套名字）。
3. 结论写在页首；细节和文件路径放后面。
4. 密钥、token、`auth.json` 内容、用户私信 **禁止** 写入 wiki。
5. 新增模块后：更新 `wiki/modules/<name>.md` + `wiki/index.md` + `wiki/log.md`。
6. 用户可见品牌名统一为 **GrokGo**；历史名 Grok Proxy 仅作兼容说明。
7. 链接优先用 `[[wikilink]]`（相对 `wiki/` 的路径，无扩展名）。

## 与其他文档的关系

| 文档 | 职责 |
|---|---|
| 仓库 `README.md` | 用户安装与卖点 |
| 仓库 `AGENTS.md` | Agent 工作区短规则 + 指向本 wiki / agents-guide |
| `~/.grok-go/agents-guide.md` | 运行时 MCP 工具参数（随 app 版本更新，勿手改） |
| `docs/BUILD.md` | 构建与发布细节 |
| `llm-wiki/` | 给 Agent 的长期项目理解层 |

## 禁止事项

- 不要把 `node_modules`、`target`、日志原文、大量上游 API 响应 dump 进 wiki。
- 不要把未验证猜测写成确定事实；不确定写「待验证」。
- 不要在 wiki 里复制完整 OAuth client secret 类敏感信息（client_id 若已在开源源码默认配置中出现，可引用文件路径说明，不要额外扩散）。
