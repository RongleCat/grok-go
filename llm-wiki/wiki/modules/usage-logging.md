# 模块：用量与日志

## 结论

`usage.rs` 用 SQLite（`~/.grok-go/data.db`）异步写入请求日志，支持今日汇总、最近日志、GitHub 风格热力图。写失败不能拖慢代理：队列满则丢日志。

## 数据

`RequestLog` 关键字段：

- request_id、account_id、endpoint
- requested_model / resolved_model
- status_code、latency_ms、first_token_ms
- input/output/cache tokens、estimated_cost_usd
- error_summary、client_source、created_at

## 运维参数（代码常量）

- 队列容量 2048
- 最多保留 50000 行
- 默认保留 30 天
- 每 64 次插入 prune + WAL checkpoint

## 空库 / 首次启动

- 启动时先 `UsageStore::open_default()` 建表，再启动异步 writer
- 空表 `SUM` 使用 `COALESCE`；UI 查询失败时降级为空汇总 / 空列表，避免 Overview 整页报错

## UI

- Overview：今日摘要
- Usage：热力图
- Logs：分页/最近列表、可清空

## 相关页面

- [[gateway]]
- [[frontend-ui]]

## 来源

- `src-tauri/src/usage.rs`
