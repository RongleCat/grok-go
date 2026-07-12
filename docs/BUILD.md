# GrokGo 桌面端构建与发布

支持平台：

| 平台 | Triple | 本地构建 | CI Release |
|------|--------|----------|------------|
| macOS Apple Silicon | `aarch64-apple-darwin` | ✅ | ✅ `macos-latest` |
| macOS Intel | `x86_64-apple-darwin` | ✅（在 Apple Silicon 上交叉） | ✅ `macos-latest` + target |
| Windows x64 | `x86_64-pc-windows-msvc` | ✅（需 Windows 主机） | ✅ `windows-latest` |

> 不在 macOS/Linux 上交叉编译 Windows Tauri 安装包（NSIS/MSI + WebView2）。Windows 产物以 CI 为准。

## 1. 本地环境

```bash
# 依赖：Node 22+、pnpm 9、Rust stable、Xcode CLT (macOS)
pnpm install
./scripts/setup-cross-compile.sh
```

### macOS

- Xcode Command Line Tools：`xcode-select --install`
- Apple Silicon 上构建 Intel：`rustup target add x86_64-apple-darwin`（脚本已处理）

### Windows

- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)（C++ 工作负载）
- [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)（多数 Win10/11 已带）
- Rust MSVC toolchain：`rustup default stable-x86_64-pc-windows-msvc`

## 2. 本地构建命令

```bash
pnpm build              # 当前主机默认 target
pnpm build:mac-arm      # macOS ARM
pnpm build:mac-intel    # macOS Intel
pnpm build:mac-all      # ARM + Intel（仅 macOS）
pnpm build:win          # Windows（仅 Windows 主机）

# 或直接：
./scripts/build-local.sh mac-arm
./scripts/build-local.sh all-mac
```

产物目录：

```
src-tauri/target/<triple>/release/bundle/
  macos/   # .app / .dmg
  nsis/    # Windows installer
  msi/
```

## 3. GitHub Actions 发布（推荐）

工作流：`.github/workflows/release.yml`

### 触发方式

1. **推送版本 tag**（推荐、稳定）  
   ```bash
   ./scripts/release-tag.sh 0.1.1
   git push origin HEAD && git push origin v0.1.1
   ```
2. **Actions → release → Run workflow**（手动）

### 仓库设置

- **Settings → Actions → General → Workflow permissions**  
  勾选 **Read and write permissions**（用于创建 Release 并上传资产）

### 可选：签名 Secrets

未配置签名时仍会出包；macOS 可能提示「已损坏」，Windows 可能 SmartScreen 拦截。

| Secret | 用途 |
|--------|------|
| `APPLE_CERTIFICATE` 等 | Apple 公证 / 签名（见 [Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/)） |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater 签名（若启用） |

### Release 内容

矩阵会为以下平台上传安装包到同一 GitHub Release：

- macOS ARM64  
- macOS x64  
- Windows x64  

## 4. 版本号约定

保持三处一致（`release-tag.sh` 会自动改）：

- `package.json` → `version`
- `src-tauri/tauri.conf.json` → `version`
- `src-tauri/Cargo.toml` → `[package].version`

Tag 格式：`v0.1.1`（前缀 `v` + semver）。

## 5. 故障排查

| 现象 | 处理 |
|------|------|
| CI “Resource not accessible by integration” | 打开 workflow 写权限 |
| macOS Intel build 缺 target | 确认 rustup 安装了 `x86_64-apple-darwin` |
| Windows 本机无法交叉 | 使用 CI 或 Windows 机器 |
| macOS 下载后打不开 | 未签名：系统设置 → 隐私与安全性 → 仍要打开；或 `xattr -cr /path/to/GrokGo.app` |
