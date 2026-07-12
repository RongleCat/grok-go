# Contributing to GrokGo

感谢关注 **GrokGo**！欢迎 Issue、PR 与使用反馈。

Thanks for your interest in contributing.

## 开发环境 / Development

```bash
pnpm install
pnpm tauri dev
```

Frontend only:

```bash
pnpm dev:ui
```

Checks:

```bash
pnpm build:ui
cd src-tauri && cargo check
```

## 贡献流程 / Workflow

1. Fork 本仓库并创建分支
2. 做尽量小而清晰的改动
3. 本地通过 `pnpm build:ui` 与 `cargo check`
4. 提交 PR，说明动机、改动与验证方式

## 约定 / Guidelines

- 用户可见文案保持 **GrokGo** 品牌一致
- 配置目录为 `~/.grok-go`
- MCP 键名为 `mcp_servers.grok-go`
- 不要提交 `node_modules`、`target`、`dist`、本地 token / `auth.json`
- 安全相关问题请走 [SECURITY.md](./SECURITY.md)

## 交流 / Contact

- X: [@cgnot996](https://x.com/cgnot996)
- GitHub Issues: https://github.com/RongleCat/grok-go/issues
