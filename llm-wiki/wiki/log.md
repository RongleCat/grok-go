# Wiki 日志

## 2026-07-12（hotfix Windows JSON）

- 分支 `hotfix/windows-json-config-load`：修复 Windows 下概览/账号页 `expected value at line 1 column 1`。
- 根因：`config.json`/`auth.json` 空文件、BOM 或损坏时 `serde_json::from_str` 硬失败，Tauri invoke 错误直出 UI。
- 处理：加载可恢复（备份坏文件 + 默认重建）+ 原子写盘；单测覆盖 empty/BOM/invalid/atomic overwrite。

## 2026-07-12

- 初始化项目 `llm-wiki`：建立 SCHEMA、raw 来源索引、核心 synthesis / modules / concepts / playbooks / queries。
- 目标：任意 Agent 接手都能从索引理解 GrokGo 是本地 Grok 网关，以及改代码应落在哪些模块。
- 基线版本：仓库 `0.1.1`。

## 2026-07-12（修复）

- 运行时 `agents-guide.md` 只渲染当前启用的 MCP 工具；与仓库开发用 `AGENTS.md` 隔离。
- 用量库空表 `SUM` 空值导致首次打开 Overview/Usage 失败：`COALESCE` + 查询/打开降级 + schema 先于 writer 初始化。
- CC Switch 导入在 MCP 已开启时写入 provider TOML 的 `mcp_servers.grok-go`，并 upsert `mcp_servers` 表。
