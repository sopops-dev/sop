#!/bin/sh
# Install sop, the SOP compiler: one prebuilt binary from GitHub Releases. No Python or uv needed.
#
#   curl -fsSL https://raw.githubusercontent.com/sopops-dev/sop/main/install.sh | sh
#
# Options (environment variables):
#   SOP_REF=<tag>               install a specific release, e.g. v0.1.0 (default: the latest release)
#   SOP_INSTALL_DIR=<dir>       where to put the binary (default: ~/.local/bin)
#
# Installs `sop` from a release's sop_<os>_<arch> archive (v0.1.0 and later).
#
# To build from a branch or commit instead, use Rust: cargo install --git https://github.com/sopops-dev/sop --branch <branch> (or --rev <commit>)

set -eu

REPO="sopops-dev/sop"
REF="${SOP_REF:-latest}"
INSTALL_DIR="${SOP_INSTALL_DIR:-$HOME/.local/bin}"
# For testing against local release archives (and checksums.txt): SOP_RELEASES_URL=file:///path/to/dist
RELEASES_URL="${SOP_RELEASES_URL:-}"

say() { printf 'sop: %s\n' "$1"; }
fail() { printf 'sop: error: %s\n' "$1" >&2; exit 1; }

case "$(uname -s)" in
  Linux) OS=linux ;;
  Darwin) OS=darwin ;;
  MINGW* | MSYS* | CYGWIN*) OS=windows ;;
  *) fail "unsupported operating system: $(uname -s). Download a binary from https://github.com/$REPO/releases" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) ARCH=amd64 ;;
  arm64 | aarch64) ARCH=arm64 ;;
  *) fail "unsupported CPU architecture: $(uname -m). Download a binary from https://github.com/$REPO/releases" ;;
esac

EXT=tar.gz
EXE=
if [ "$OS" = windows ]; then
  EXT=zip
  EXE=.exe
fi

case "$REF" in
  latest | v[0-9]*) ;;
  [0-9]*) REF="v$REF" ;;
  *) fail "SOP_REF must be a release tag like v0.1.0 (got '$REF'). To build a branch or commit: cargo install --git https://github.com/$REPO --branch $REF (a commit: --rev $REF)" ;;
esac
case "$REF" in
  v0.0.*) fail "releases before v0.1.0 are named sopc; install one from https://github.com/$REPO/releases" ;;
esac

if [ -n "$RELEASES_URL" ]; then
  BASE="$RELEASES_URL"
elif [ "$REF" = latest ]; then
  BASE="https://github.com/$REPO/releases/latest/download"
else
  BASE="https://github.com/$REPO/releases/download/$REF"
fi

download() { # url dest
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$1" -O "$2"
  else
    fail "need curl or wget to download sop"
  fi
}

TMP="$(mktemp -d 2>/dev/null || mktemp -d -t sop)"
trap 'rm -rf "$TMP"' EXIT INT TERM

ASSET="sop_${OS}_${ARCH}.${EXT}"
say "downloading $ASSET ($REF)"
if ! download "$BASE/$ASSET" "$TMP/$ASSET" 2>/dev/null; then
  if [ "$REF" = latest ]; then
    fail "no sop release found at https://github.com/$REPO/releases (none published yet?). Build from source instead: cargo install --git https://github.com/$REPO"
  else
    fail "release $REF not found, or it has no $ASSET. See https://github.com/$REPO/releases for available versions"
  fi
fi
BIN="sop$EXE"

# Verify the checksum when the release has one and a sha256 tool is available.
if download "$BASE/checksums.txt" "$TMP/checksums.txt" 2>/dev/null; then
  EXPECTED="$(grep " $ASSET\$" "$TMP/checksums.txt" | cut -d ' ' -f 1 || true)"
  ACTUAL=""
  if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL="$(sha256sum "$TMP/$ASSET" | cut -d ' ' -f 1)"
  elif command -v shasum >/dev/null 2>&1; then
    ACTUAL="$(shasum -a 256 "$TMP/$ASSET" | cut -d ' ' -f 1)"
  fi
  if [ -n "$EXPECTED" ] && [ -n "$ACTUAL" ] && [ "$EXPECTED" != "$ACTUAL" ]; then
    fail "checksum mismatch for $ASSET; try again, or download it by hand from https://github.com/$REPO/releases"
  fi
fi

mkdir -p "$TMP/x"
if [ "$EXT" = zip ]; then
  command -v unzip >/dev/null 2>&1 || fail "need unzip to extract $ASSET"
  unzip -q "$TMP/$ASSET" -d "$TMP/x"
else
  tar -xzf "$TMP/$ASSET" -C "$TMP/x"
fi
[ -f "$TMP/x/$BIN" ] || fail "$ASSET doesn't contain $BIN"

mkdir -p "$INSTALL_DIR"
cp "$TMP/x/$BIN" "$INSTALL_DIR/$BIN.tmp"
chmod 755 "$INSTALL_DIR/$BIN.tmp"
mv "$INSTALL_DIR/$BIN.tmp" "$INSTALL_DIR/$BIN"
say "installed $INSTALL_DIR/$BIN"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    say "$INSTALL_DIR isn't on your PATH yet. Add it, then open a new terminal:"
    say "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.profile"
    ;;
esac

say "next: 'sop skills install' in your repo, then run the sop-import skill in Claude Code, Codex or OpenCode"
