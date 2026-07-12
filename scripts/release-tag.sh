#!/usr/bin/env bash
# Create an annotated release tag after syncing version across manifests.
# Usage:
#   ./scripts/release-tag.sh 0.1.1
#   ./scripts/release-tag.sh 0.1.1 --push
#
# Does NOT push by default. CI (.github/workflows/release.yml) runs on tag push v*.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="${1:-}"
PUSH="${2:-}"

if [[ -z "$VERSION" ]]; then
  echo "usage: $0 <semver> [--push]" >&2
  echo "example: $0 0.1.1 --push" >&2
  exit 1
fi

# strip leading v if provided
VERSION="${VERSION#v}"
TAG="v${VERSION}"

if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-].*)?$ ]]; then
  echo "error: invalid semver: $VERSION" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "error: working tree not clean. Commit or stash first." >&2
  git status -sb
  exit 1
fi

if git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "error: tag already exists: $TAG" >&2
  exit 1
fi

echo "==> Bumping version to $VERSION"

# package.json
python3 - <<PY
import json
from pathlib import Path
p = Path("package.json")
data = json.loads(p.read_text())
data["version"] = "$VERSION"
p.write_text(json.dumps(data, indent=2) + "\n")
print("package.json ->", data["version"])
PY

# tauri.conf.json
python3 - <<PY
import json
from pathlib import Path
p = Path("src-tauri/tauri.conf.json")
data = json.loads(p.read_text())
data["version"] = "$VERSION"
p.write_text(json.dumps(data, indent=2) + "\n")
print("tauri.conf.json ->", data["version"])
PY

# Cargo.toml package version only (first occurrence under [package])
python3 - <<PY
from pathlib import Path
import re
p = Path("src-tauri/Cargo.toml")
text = p.read_text()
new, n = re.subn(
    r'(?m)^version\s*=\s*"[^"]+"',
    'version = "$VERSION"',
    text,
    count=1,
)
if n != 1:
    raise SystemExit(f"expected 1 package version line, got {n}")
p.write_text(new)
print("Cargo.toml -> $VERSION")
PY

git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml
if [[ -n "$(git status --porcelain)" ]]; then
  git commit -m "chore: release $TAG"
fi

git tag -a "$TAG" -m "Release $TAG"
echo "Created tag $TAG"

if [[ "$PUSH" == "--push" ]]; then
  git push origin HEAD
  git push origin "$TAG"
  echo "Pushed $TAG — GitHub Actions release workflow should start."
else
  echo "Tag is local only. Push when ready:"
  echo "  git push origin HEAD && git push origin $TAG"
fi
