# GrokGo Open-Source Renovation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Brand the project as GrokGo, add standard open-source scaffolding, bilingual README with screenshots, and point the repo at `RongleCat/grok-go`.

**Architecture:** Full-string/package rename across Tauri + React + website; no runtime legacy path support. Assets live under `assets/`. Screenshots captured from Vite UI at 1080×720. Local machine config copied once from `~/.grok-proxy` to `~/.grok-go`.

**Tech Stack:** Tauri 2, React, Vite, Tailwind, pnpm, Playwright/Puppeteer-or-Chrome headless for screenshots, GitHub markdown assets.

**Design:** `docs/plans/2026-07-12-grokgo-opensource-renovation-design.md`

---

### Task 1: Local config home + git remote foundation

**Files:**
- Machine: `~/.grok-go`
- Create/modify: `.git`, remote

**Step 1: Migrate local config directory**

```bash
if [ -d "$HOME/.grok-proxy" ] && [ ! -d "$HOME/.grok-go" ]; then
  cp -a "$HOME/.grok-proxy" "$HOME/.grok-go"
fi
ls -la "$HOME/.grok-go"
```

Expected: `~/.grok-go` contains `config.json`, `auth.json`, `data.db`, `artifacts/`, etc.

**Step 2: Initialize git if missing and set remote**

```bash
git status 2>/dev/null || git init
git remote remove origin 2>/dev/null || true
git remote add origin git@github.com:RongleCat/grok-go.git
git remote -v
```

**Step 3: Commit checkpoint only if user asked later** (do not force commits mid-plan unless requested)

---

### Task 2: Place brand logo assets

**Files:**
- Create: `assets/logo.png`
- Create: `assets/screenshots/.gitkeep`
- Source: user-provided logo image from chat attachment

**Step 1: Create assets dirs**

```bash
mkdir -p assets/screenshots
```

**Step 2: Copy logo into repo**

Locate the uploaded logo (chat image / attachments) and copy to `assets/logo.png`.  
If only path known via Codex image attachment, export/copy that PNG.

**Step 3: Verify**

```bash
file assets/logo.png
```

Expected: PNG image data.

---

### Task 3: Rename package / Tauri product metadata

**Files:**
- Modify: `package.json`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/src/main.rs` (if window/app name strings)
- Modify: `src-tauri/src/lib.rs`

**Step 1: package.json**

```json
{
  "name": "grok-go",
  "private": true,
  "version": "0.1.0"
}
```

Keep scripts unchanged.

**Step 2: Cargo.toml package**

```toml
[package]
name = "grok-go"
version = "0.1.0"
description = "GrokGo — local Grok gateway for Codex"
authors = ["RongleCat"]

[lib]
name = "grok_go_lib"
crate-type = ["staticlib", "cdylib", "rlib"]
```

**Step 3: tauri.conf.json**

```json
{
  "productName": "GrokGo",
  "version": "0.1.0",
  "identifier": "com.grokgo.desktop",
  "app": {
    "windows": [
      {
        "title": "GrokGo",
        "width": 1080,
        "height": 720
      }
    ]
  }
}
```

**Step 4: Fix Rust lib name references**

In `src-tauri/src/main.rs` / `lib.rs`, ensure `grok_go_lib` is used if previously `grok_proxy_lib`.

**Step 5: cargo check**

```bash
cd src-tauri && cargo check
```

Expected: compiles (may need `Cargo.lock` package name updates).

---

### Task 4: Rename config path and integrations identifiers

**Files:**
- Modify: `src-tauri/src/paths.rs`
- Modify: `src-tauri/src/integrations.rs`
- Modify: other rust files with `grok-proxy` / `Grok Proxy` / `~/.grok-proxy` / User-Agent strings:
  - `auth.rs`, `http_client.rs`, `gateway/server.rs`, `gateway/sanitize.rs`, `gateway/image_bridge.rs`, `gateway/media_artifacts.rs`, `lib.rs`, `main.rs`

**Step 1: paths.rs**

Change home dir join from `.grok-proxy` to `.grok-go`.

**Step 2: integrations.rs**

Replace:
- `<!-- grok-proxy:agents-guide:start -->` → `<!-- grok-go:agents-guide:start -->`
- end marker similarly
- `mcp_servers.grok-proxy` / table key `grok-proxy` → `grok-go`
- display names `Grok Proxy` → `GrokGo`
- artifact path text `~/.grok-proxy` → `~/.grok-go`
- provider names / model_provider strings to grok-go / GrokGo

**Step 3: Sweep remaining Rust strings**

```bash
rg -n "grok-proxy|Grok Proxy|grok_proxy|com\.grokproxy|\.grok-proxy" src-tauri/src
```

Replace all product identifiers. Keep technical comments historical only if necessary; prefer clean rename.

**Step 4: cargo check again**

```bash
cd src-tauri && cargo check
```

---

### Task 5: Frontend i18n + UI brand

**Files:**
- Modify: `src/i18n/index.ts` (`STORAGE_KEY` → `grok-go.locale`)
- Modify: `src/i18n/locales/zh-CN.ts`
- Modify: `src/i18n/locales/en.ts`
- Modify: `src/components/layout.tsx` (logo image instead of Activity icon)

**Step 1: zh-CN app block**

```ts
app: {
  name: "GrokGo",
  tagline: "本地 Grok 网关，即开即用",
},
```

Update MCP copy: `mcp_servers.grok-go`, messages saying grok-go.

**Step 2: en app block**

```ts
app: {
  name: "GrokGo",
  tagline: "Local Grok gateway, ready out of the box",
},
```

**Step 3: layout brand mark**

Use `/logo` or imported `assets/logo.png` (copy to `src/assets/logo.png` or `public/logo.png`) in the sidebar header.

**Step 4: rg frontend**

```bash
rg -n "Grok Proxy|grok-proxy" src
```

---

### Task 6: Regenerate app icons from logo

**Files:**
- Source: `assets/logo.png`
- Modify/generate: `src-tauri/icons/**`

**Step 1: Prefer Tauri icon generator**

```bash
pnpm tauri icon assets/logo.png
```

Expected: updates `src-tauri/icons` png/icns/ico set.

**Step 2: If tray-specific icons break, regenerate tray variants from same source** (keep existing structure under `icons/`).

**Step 3: Spot-check `icon.png`, `icon.icns`, `icon.ico` exist.

---

### Task 7: Website rename

**Files:**
- Modify: `website/package.json` name → `grok-go-site`
- Modify: `website/app/layout.tsx` metadata
- Modify: `website/app/page.tsx` all Grok Proxy copy → GrokGo
- Optional: `website/public/favicon` from logo

Replace titles/descriptions/siteName/openGraph/twitter and body copy. Slogan may appear on hero if present.

---

### Task 8: Open-source scaffolding files

**Files:**
- Create: `LICENSE` (MIT, copyright RongleCat / year 2026)
- Create: `CHANGELOG.md`
- Create: `CONTRIBUTING.md` (ZH primary, link EN section or short EN)
- Create: `CODE_OF_CONDUCT.md` (Contributor Covenant adapted)
- Create: `SECURITY.md`
- Create: `.github/ISSUE_TEMPLATE/bug_report.yml`
- Create: `.github/ISSUE_TEMPLATE/feature_request.yml`
- Create: `.github/PULL_REQUEST_TEMPLATE.md`
- Create: `.github/workflows/ci.yml`

**CI skeleton example:**

```yaml
name: ci
on:
  push:
  pull_request:
jobs:
  ui:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 9
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: pnpm
      - run: pnpm install
      - run: pnpm build:ui
  rust:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo check
        working-directory: src-tauri
```

Adjust if workspace needs extra Linux deps for Tauri (CI may only `cargo check` lib without full bundle).

---

### Task 9: Capture UI screenshots at 1080×720

**Files:**
- Create: `assets/screenshots/overview.png`
- Create: `assets/screenshots/accounts.png`
- Create: `assets/screenshots/integrations.png`
- Create: `assets/screenshots/usage.png`
- Optional: `logs.png`, `settings.png`

**Step 1: Start UI**

```bash
pnpm dev:ui
```

Default Vite/Tauri dev URL: `http://localhost:1420`

**Step 2: Headless screenshot with viewport 1080×720**

Use Playwright:

```bash
npx --yes playwright install chromium
node <<'NODE'
const { chromium } = require('playwright');
(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1080, height: 720 } });
  const base = 'http://localhost:1420';
  const shots = [
    ['/', 'assets/screenshots/overview.png'],
    ['/accounts', 'assets/screenshots/accounts.png'],
    ['/integrations', 'assets/screenshots/integrations.png'],
    ['/usage', 'assets/screenshots/usage.png'],
  ];
  for (const [path, file] of shots) {
    await page.goto(base + path, { waitUntil: 'networkidle' });
    await page.waitForTimeout(1000);
    await page.screenshot({ path: file, fullPage: false });
  }
  await browser.close();
})();
NODE
```

Note: UI without Tauri backend may show load errors; if needed run `pnpm tauri dev` and screenshot the webview URL, still forcing 1080×720 viewport. Prefer pages that still look good empty-state.

**Step 3: Verify image dimensions 1080×720**

```bash
file assets/screenshots/*.png
```

---

### Task 10: Rewrite README.md (ZH) and README_EN.md

**Files:**
- Replace: `README.md`
- Create: `README_EN.md`
- Update: `AGENTS.md` product naming
- Update: `docs/HANDOFF.md` product naming (light touch)

**README.md required sections:**

1. Centered or left logo + `# GrokGo`
2. 副标题 + slogan
3. `[English](./README_EN.md) | 中文`
4. Badges:
   - MIT
   - GitHub stars shield for `RongleCat/grok-go`
5. X CTA: 欢迎在 X 关注作者 [@cgnot996](https://x.com/cgnot996)
6. Features list (from product capabilities)
7. Screenshots grid referencing `assets/screenshots/*`
8. 快速开始
9. 接入 Codex / MCP (`mcp_servers.grok-go`)
10. 默认 endpoints、端口 8787
11. 配置目录 `~/.grok-go`
12. 技术栈
13. 贡献 / 安全 / License
14. Star history:

```markdown
## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=RongleCat/grok-go&type=Date)](https://star-history.com/#RongleCat/grok-go&Date)
```

15. Footer X CTA again

**README_EN.md:** mirror structure in English; link back to `README.md`.

**Do not** document old-path migration for users.

---

### Task 11: Docs / AGENTS sweep

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/HANDOFF.md` (rename product strings; keep history honest)
- Optional leave `docs/plans/2026-07-10-grok-proxy-desktop-design.md` filename; update title note that product is now GrokGo if editing

```bash
rg -n "Grok Proxy|grok-proxy|GrokProxy" -g '!node_modules' -g '!target' -g '!dist' -g '!docs/plans/2026-07-10*' -g '!pnpm-lock.yaml' -g '!Cargo.lock'
```

Fix remaining public/product strings.

---

### Task 12: Final verification

**Step 1: Frontend build**

```bash
pnpm build:ui
```

**Step 2: Rust check**

```bash
cd src-tauri && cargo check
```

**Step 3: Naming audit**

```bash
rg -n "Grok Proxy|productName.*Proxy|mcp_servers\.grok-proxy|\.grok-proxy" -g '!node_modules' -g '!target' -g '!dist' -g '!Cargo.lock' -g '!pnpm-lock.yaml' -g '!docs/plans/*'
```

Expected: clean for product paths (historical design filename OK).

**Step 4: Confirm local config**

```bash
test -d "$HOME/.grok-go" && test -f "$HOME/.grok-go/config.json" && echo OK
```

**Step 5: Confirm assets**

```bash
test -f assets/logo.png && ls assets/screenshots
```

---

## Execution notes

- Prefer ASCII in code identifiers (`grok-go`, `GrokGo`).
- Do not add runtime migration code.
- Screenshot viewport must be **1080×720**, not fullPage desktop retina unless scaled to that logical size.
- If Playwright cannot talk to Tauri IPC, capture polished empty/error-tolerant UI states still useful for README.
- MIT year: 2026; author display: RongleCat / X @cgnot996.

## Done when

- [ ] GrokGo naming everywhere user-facing
- [ ] MIT + bilingual README + community files + CI skeleton
- [ ] Logo + 1080×720 screenshots in `assets/`
- [ ] `~/.grok-go` populated on this machine
- [ ] `cargo check` + `pnpm build:ui` pass
- [ ] Remote `origin` = `git@github.com:RongleCat/grok-go.git`
