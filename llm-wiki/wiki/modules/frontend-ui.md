# 模块：前端 UI

## 结论

前端是薄控制台：路由在 `src/App.tsx`，数据几乎全部经 `src/lib/api.ts` 调 Tauri commands。业务真相在 Rust 侧。

## 页面

| 路由 | 页面 | 职责 |
|---|---|---|
| `/` | Overview | 运行状态、Base URL、token、今日用量 |
| `/accounts` | Accounts | 多账号 OAuth、批量导入（CPA/sub2api/SSO）、多选批量改删、图/视能力开关、权重、cooldown |
| `/mapping` | Mapping | 模型映射表 |
| `/integrations` | Integrations | Codex / MCP / CC Switch / Grok Build |
| `/usage` | Usage | 汇总 + 热力图 |
| `/logs` | Logs | 最近请求日志 |
| `/settings` | Settings | 端口/局域网/代理/默认模型/备份/图标等 |

## 关键文件

- `src/lib/api.ts`：类型与 `invoke` 封装
- `src/components/layout.tsx`：壳布局
- `src/i18n/*`：中英文案
- `src/components/heatmap.tsx`：用量热力

## 开发注意

- 新增 Tauri command 必须同时：
  1. `commands.rs` 实现
  2. `lib.rs` `generate_handler!` 注册
  3. `api.ts` 封装
  4. UI 使用
- 用户可见品牌统一 **GrokGo**
- 配置目录文案统一 `~/.grok-go`

## 相关页面

- [[config-runtime]]
- [[integrations]]
- [[usage-logging]]

## 来源

- `src/App.tsx`
- `src/lib/api.ts`
- `src/pages/*`
