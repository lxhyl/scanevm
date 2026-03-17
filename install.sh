#!/usr/bin/env sh
# Install or update etherscan-cli to the latest release.
# Usage: curl -fsSL https://raw.githubusercontent.com/lxhyl/etherscan-cli/main/install.sh | sh

set -e

REPO="lxhyl/etherscan-cli"
BIN="etherscan"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

# Detect OS and arch
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  linux)
    case "$ARCH" in
      x86_64)  TARGET="x86_64-unknown-linux-musl" ;;
      aarch64) TARGET="aarch64-unknown-linux-musl" ;;
      arm64)   TARGET="aarch64-unknown-linux-musl" ;;
      *) echo "Unsupported arch: $ARCH"; exit 1 ;;
    esac
    EXT="tar.gz"
    ;;
  darwin)
    case "$ARCH" in
      x86_64) TARGET="x86_64-apple-darwin" ;;
      arm64)  TARGET="aarch64-apple-darwin" ;;
      *) echo "Unsupported arch: $ARCH"; exit 1 ;;
    esac
    EXT="tar.gz"
    ;;
  *)
    echo "Unsupported OS: $OS"
    echo "On Windows, download from: https://github.com/$REPO/releases/latest"
    exit 1
    ;;
esac

# Get latest version
LATEST=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
  | grep '"tag_name"' | sed 's/.*"tag_name": "\(.*\)".*/\1/')

if [ -z "$LATEST" ]; then
  echo "Failed to fetch latest release version"
  exit 1
fi

# Check if already up to date
CURRENT=""
if command -v "$BIN" >/dev/null 2>&1; then
  CURRENT=$("$BIN" --version 2>/dev/null | awk '{print $NF}' || true)
  CURRENT="v$CURRENT"
fi

if [ "$CURRENT" = "$LATEST" ]; then
  echo "$BIN is already up to date ($LATEST)"
  exit 0
fi

if [ -n "$CURRENT" ]; then
  echo "Updating $BIN $CURRENT -> $LATEST"
else
  echo "Installing $BIN $LATEST"
fi

# Download and install
URL="https://github.com/$REPO/releases/download/$LATEST/${BIN}-${LATEST}-${TARGET}.${EXT}"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "Downloading $URL"
curl -fsSL "$URL" -o "$TMP/archive.$EXT"

cd "$TMP"
tar -xzf "archive.$EXT"

# Make install dir writable (may need sudo)
if [ -w "$INSTALL_DIR" ]; then
  mv "$BIN" "$INSTALL_DIR/$BIN"
else
  echo "Installing to $INSTALL_DIR (may prompt for sudo)"
  sudo mv "$BIN" "$INSTALL_DIR/$BIN"
fi

chmod +x "$INSTALL_DIR/$BIN"
echo "Installed $BIN $LATEST to $INSTALL_DIR/$BIN"
