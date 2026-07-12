# Playbook：开发与发布

## 开发

```bash
pnpm install
pnpm tauri dev          # 桌面 + 网关
pnpm dev:ui             # 仅前端
pnpm build:ui
cd src-tauri && cargo check && cargo test --lib
```

环境：Node 22+、pnpm 9、Rust stable；macOS 需 Xcode CLT。

## 本地出包

```bash
pnpm build
pnpm build:mac-arm
pnpm build:mac-intel
pnpm build:mac-all
# Windows 安装包请在 Windows 或 CI 构建
```

产物：`src-tauri/target/<triple>/release/bundle/`

## 发版

1. 三处 version 对齐（`scripts/release-tag.sh` 可改）
2. `./scripts/release-tag.sh 0.1.x`
3. push commit + tag `v0.1.x`
4. GitHub Actions `release.yml` 打 macOS ARM/Intel + Windows x64

详见 `docs/BUILD.md`。

## 相关页面

- [[agent-onboarding]]
- [[../syntheses/project-overview]]

## 来源

- `docs/BUILD.md`
- `package.json`
- `.github/workflows/*`
