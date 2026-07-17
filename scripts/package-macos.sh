#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This packaging script only supports macOS." >&2
  exit 1
fi

for command in npm cargo ditto; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "Missing required command: $command" >&2
    echo "See README.md → Installation for setup instructions." >&2
    exit 1
  fi
done

if [[ ! -d node_modules ]]; then
  echo "Frontend dependencies are missing. Run: npm ci" >&2
  exit 1
fi

npm run check
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
npm run app:build

APP_PATH="$ROOT_DIR/src-tauri/target/release/bundle/macos/Sticky.app"
ARCH="$(uname -m)"
OUTPUT_DIR="$ROOT_DIR/dist"
OUTPUT_PATH="$OUTPUT_DIR/Sticky-macOS-$ARCH.zip"

mkdir -p "$OUTPUT_DIR"
rm -f "$OUTPUT_PATH"
ditto -c -k --sequesterRsrc --keepParent "$APP_PATH" "$OUTPUT_PATH"

echo
echo "Package ready: $OUTPUT_PATH"
echo "Upload that ZIP to a release, or unzip it and drag Sticky.app into Applications."
