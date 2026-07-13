#!/usr/bin/env python3
"""Extract a version section from CHANGELOG.md for GitHub Release body.

Usage:
  python3 scripts/changelog-for-release.py 0.1.5
  python3 scripts/changelog-for-release.py v0.1.5

Output: Markdown to stdout (assets table + changelog section + install notes).
Exit 1 if the version section is missing (fail the release job intentionally).
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CHANGELOG = ROOT / "CHANGELOG.md"

ASSETS_TABLE = """## Downloads / 下载

| Platform | File (typical) |
|----------|----------------|
| macOS Apple Silicon | `*.dmg` (aarch64) |
| macOS Intel | `*.dmg` (x64) |
| Windows x64 | `*-setup.exe` / `*.msi` |

Full notes below (from `CHANGELOG.md`). 完整说明见下方更新日志。
"""

INSTALL_NOTES = """
---

### Install notes / 安装说明

- **macOS** (unsigned / not notarized): if Gatekeeper blocks the app, run `xattr -cr /Applications/GrokGo.app`, or right-click → Open. See README.
- **Windows**: SmartScreen may warn until code signing is configured.
- Changelog source: [`CHANGELOG.md`](https://github.com/RongleCat/grok-go/blob/main/CHANGELOG.md)
"""


def normalize_version(raw: str) -> str:
    v = raw.strip()
    if v.startswith("v") or v.startswith("V"):
        v = v[1:]
    return v


def extract_section(text: str, version: str) -> str | None:
    """Return body under ## [version] ... until next ## [ or EOF."""
    # Allow optional date suffix: ## [0.1.4] - 2026-07-13
    pat = re.compile(
        rf"^## \[{re.escape(version)}\][^\n]*\n(.*?)(?=^## \[|\Z)",
        re.MULTILINE | re.DOTALL,
    )
    m = pat.search(text)
    if not m:
        return None
    body = m.group(1).strip()
    return body


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: changelog-for-release.py <semver|vX.Y.Z>", file=sys.stderr)
        return 2
    version = normalize_version(sys.argv[1])
    if not CHANGELOG.is_file():
        print(f"error: missing {CHANGELOG}", file=sys.stderr)
        return 1
    text = CHANGELOG.read_text(encoding="utf-8")
    section = extract_section(text, version)
    if not section:
        print(
            f"error: no CHANGELOG section for [{version}]. "
            f"Add `## [{version}] - YYYY-MM-DD` before tagging.",
            file=sys.stderr,
        )
        return 1

    header = f"# GrokGo v{version}\n\n"
    out = (
        header
        + ASSETS_TABLE
        + "\n"
        + f"## Changelog / 更新日志 — v{version}\n\n"
        + section
        + "\n"
        + INSTALL_NOTES
    )
    sys.stdout.write(out)
    if not out.endswith("\n"):
        sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
