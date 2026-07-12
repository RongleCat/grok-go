#!/usr/bin/env bash
# Install Rust targets used by GrokGo desktop builds.
# Run once on each developer machine (or after rustup update).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v rustup >/dev/null 2>&1; then
  echo "error: rustup not found. Install from https://rustup.rs" >&2
  exit 1
fi

echo "==> Ensuring stable toolchain"
rustup toolchain install stable
rustup default stable

HOST="$(rustc -vV | sed -n 's/^host: //p')"
echo "Host triple: $HOST"

# Desktop targets we care about for local / CI parity.
TARGETS=(
  "aarch64-apple-darwin" # macOS Apple Silicon
  "x86_64-apple-darwin"  # macOS Intel
  "x86_64-pc-windows-msvc" # Windows x64 (build on Windows or CI)
)

for t in "${TARGETS[@]}"; do
  echo "==> rustup target add $t"
  rustup target add "$t" || {
    echo "warn: failed to add $t (ok if unsupported on this host)" >&2
  }
done

if [[ "$(uname -s)" == "Darwin" ]]; then
  echo "==> macOS: Xcode CLT check"
  xcode-select -p >/dev/null 2>&1 || {
    echo "warn: Xcode Command Line Tools missing — run: xcode-select --install" >&2
  }
fi

if [[ "$(uname -s)" == "MINGW"* ]] || [[ "$(uname -s)" == "MSYS"* ]] || [[ "$(uname -s)" == "CYGWIN"* ]] || [[ -n "${WINDIR:-}" && "$(uname -o 2>/dev/null || true)" == "Msys" ]]; then
  echo "==> Windows host detected"
  echo "    Ensure WebView2 + Visual Studio Build Tools (C++/MSVC) are installed."
fi

echo ""
echo "Done. Useful commands:"
echo "  pnpm build:mac-arm      # aarch64-apple-darwin"
echo "  pnpm build:mac-intel   # x86_64-apple-darwin (on Apple Silicon needs Rosetta tooling)"
echo "  pnpm build:win         # x86_64-pc-windows-msvc (must run on Windows)"
echo "  pnpm build             # host default"
echo ""
echo "Windows .exe/.msi cannot be reliably cross-built from macOS; use GitHub Actions or a Windows machine."
