#!/usr/bin/env bash
# Local release-style builds for supported desktop targets.
# Usage:
#   ./scripts/build-local.sh              # host default
#   ./scripts/build-local.sh mac-arm
#   ./scripts/build-local.sh mac-intel
#   ./scripts/build-local.sh win          # Windows host only
#   ./scripts/build-local.sh all-mac      # both mac targets (Darwin only)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TARGET_ALIAS="${1:-host}"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: missing command: $1" >&2
    exit 1
  }
}

need_cmd pnpm
need_cmd rustc
need_cmd cargo

OS="$(uname -s)"

build_target() {
  local triple="$1"
  echo ""
  echo "======== Building target: $triple ========"
  rustup target add "$triple" >/dev/null 2>&1 || true
  # Frontend is built via beforeBuildCommand in tauri.conf.json
  pnpm exec tauri build --target "$triple"
  echo "Artifacts under: src-tauri/target/${triple}/release/bundle/"
}

case "$TARGET_ALIAS" in
  host|"")
    echo "======== Building host default ========"
    pnpm exec tauri build
    echo "Artifacts under: src-tauri/target/release/bundle/"
    ;;
  mac-arm|aarch64-apple-darwin)
    if [[ "$OS" != "Darwin" ]]; then
      echo "error: mac-arm builds require macOS" >&2
      exit 1
    fi
    build_target "aarch64-apple-darwin"
    ;;
  mac-intel|x86_64-apple-darwin)
    if [[ "$OS" != "Darwin" ]]; then
      echo "error: mac-intel builds require macOS" >&2
      exit 1
    fi
    build_target "x86_64-apple-darwin"
    ;;
  win|windows|x86_64-pc-windows-msvc)
    if [[ "$OS" == "Darwin" ]] || [[ "$OS" == "Linux" ]]; then
      echo "error: Windows Tauri bundles (NSIS/MSI + WebView2) must be built on Windows or CI." >&2
      echo "       Push a version tag (v*) to run .github/workflows/release.yml" >&2
      exit 1
    fi
    build_target "x86_64-pc-windows-msvc"
    ;;
  all-mac)
    if [[ "$OS" != "Darwin" ]]; then
      echo "error: all-mac requires macOS" >&2
      exit 1
    fi
    build_target "aarch64-apple-darwin"
    build_target "x86_64-apple-darwin"
    ;;
  *)
    echo "usage: $0 [host|mac-arm|mac-intel|win|all-mac]" >&2
    exit 1
    ;;
esac

echo ""
echo "Build finished for: $TARGET_ALIAS"
