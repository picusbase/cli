#!/usr/bin/env bash
set -euo pipefail

REPO="picusbase/cli"
BIN="picusbase"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

detect_target() {
  local os arch
  os=$(uname -s)
  arch=$(uname -m)
  case "$os" in
    Linux)
      case "$arch" in
        x86_64)  echo "linux-x86_64" ;;
        aarch64) echo "linux-aarch64" ;;
        *) echo "unsupported arch: $arch" >&2; exit 1 ;;
      esac ;;
    Darwin)
      case "$arch" in
        x86_64)  echo "macos-x86_64" ;;
        arm64)   echo "macos-aarch64" ;;
        *) echo "unsupported arch: $arch" >&2; exit 1 ;;
      esac ;;
    *) echo "unsupported OS: $os" >&2; exit 1 ;;
  esac
}

TARGET=$(detect_target)
LATEST=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | cut -d'"' -f4)
URL="https://github.com/${REPO}/releases/download/${LATEST}/${BIN}-${TARGET}"

echo "Installing picusbase ${LATEST} (${TARGET}) → ${INSTALL_DIR}/${BIN}"
curl -fsSL "$URL" -o "/tmp/${BIN}"
chmod +x "/tmp/${BIN}"

if [ -w "$INSTALL_DIR" ]; then
  mv "/tmp/${BIN}" "${INSTALL_DIR}/${BIN}"
else
  sudo mv "/tmp/${BIN}" "${INSTALL_DIR}/${BIN}"
fi

echo "Done. Run: picusbase --help"
