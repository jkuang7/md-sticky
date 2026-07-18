#!/usr/bin/env bash

set -euo pipefail

REPOSITORY="https://github.com/jkuang7/StickyMD.git"
INSTALL_DIR="$HOME/StickyMD"

fail() {
  printf '\nSticky was not installed: %s\n' "$1" >&2
  exit 1
}

[[ "$(/usr/bin/uname -s)" == "Darwin" ]] || fail "this installer only supports macOS."
[[ "$(/usr/bin/uname -m)" == "arm64" ]] || fail "this installer requires an Apple Silicon Mac."

if ! /usr/bin/xcrun --find clang >/dev/null 2>&1; then
  printf "Your Mac needs Apple's free developer tools.\n"
  /usr/bin/xcode-select --install >/dev/null 2>&1 || true
  printf 'Click Install in the window that appears and wait for it to finish.\n'
  read -r -p 'Then return here and press Return: '
fi

/usr/bin/xcrun --find clang >/dev/null 2>&1 || \
  fail "Apple's developer tools are still unavailable. Run this command again after they finish installing."
command -v git >/dev/null 2>&1 || fail "Git is still unavailable. Restart Terminal and run this command again."

if [[ -e "$INSTALL_DIR" && ! -d "$INSTALL_DIR/.git" ]]; then
  fail "$INSTALL_DIR already exists but is not a StickyMD checkout. Move or rename it, then run this command again."
fi

if [[ -d "$INSTALL_DIR/.git" ]]; then
  origin="$(git -C "$INSTALL_DIR" remote get-url origin 2>/dev/null || true)"
  case "$origin" in
    "$REPOSITORY"|git@github.com:jkuang7/StickyMD.git)
      ;;
    *)
      fail "$INSTALL_DIR belongs to a different repository. Move or rename it, then run this command again."
      ;;
  esac
  printf 'Updating StickyMD...\n'
  git -C "$INSTALL_DIR" pull --ff-only || \
    fail "StickyMD could not be updated. Check the message above."
else
  printf 'Downloading StickyMD...\n'
  git clone "$REPOSITORY" "$INSTALL_DIR" || \
    fail "StickyMD could not be downloaded. Check your internet connection."
fi

exec "$INSTALL_DIR/install.sh"
