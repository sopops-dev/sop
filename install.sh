#!/bin/sh
# Install sop, the SOP compiler: one prebuilt binary from GitHub Releases. No Python or uv needed.
#
#   curl -fsSL https://raw.githubusercontent.com/sopops-dev/sop/main/install.sh | sh
#
# Options (environment variables):
#   SOP_REF=<tag>               install a specific release, e.g. v0.0.18 (default: the latest release)
#   SOP_INSTALL_DIR=<dir>       where to put the binary (default: ~/.local/bin)
#
# The names from before the CLI was renamed from sopc to sop (SOPC_REF, SOPC_INSTALL_DIR,
# SOPC_RELEASES_URL) still work when the new ones aren't set. Releases from before the rename
# (v0.0.18 and earlier) ship sopc_<os>_<arch> archives and are installed as `sopc`, as before;
# later ones ship sop_<os>_<arch> and are installed as `sop`.
#
# To build from a branch or commit instead, use Rust: cargo install --git https://github.com/sopops-dev/sop --branch <branch> (or --rev <commit>)

set -eu

REPO="sopops-dev/sop"
REF="${SOP_REF:-${SOPC_REF:-latest}}"
INSTALL_DIR="${SOP_INSTALL_DIR:-${SOPC_INSTALL_DIR:-$HOME/.local/bin}}"
# For testing against local release archives (and checksums.txt): SOP_RELEASES_URL=file:///path/to/dist
RELEASES_URL="${SOP_RELEASES_URL:-${SOPC_RELEASES_URL:-}}"

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
  *) fail "SOP_REF must be a release tag like v0.0.18 (got '$REF'). To build a branch or commit: cargo install --git https://github.com/$REPO --branch $REF (a commit: --rev $REF)" ;;
esac

# Each kind of release archive: <archive name prefix>:<binary inside it>:<name to install it as>.
#   sop      releases after the rename from sopc: a `sop` binary
#   sopc     v0.0.6 to v0.0.18: a `sopc` binary, installed as sopc as before (the sopops
#            Dockerfile pins one of these and expects sopc)
#   opensop  before v0.0.6 (when the project was named OpenSOP): an `opensop` binary, installed
#            as sopc
# The kinds are tried in order until a release has the archive; known old tags start with sopc.
case "$REF" in
  v0.0.[0-9] | v0.0.1[0-8]) KINDS="sopc:sopc:sopc opensop:opensop:sopc sop:sop:sop" ;;
  *) KINDS="sop:sop:sop sopc:sopc:sopc opensop:opensop:sopc" ;;
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

ASSET=
FIRST=
for kind in $KINDS; do
  prefix="${kind%%:*}"
  rest="${kind#*:}"
  candidate="${prefix}_${OS}_${ARCH}.${EXT}"
  if [ -z "$FIRST" ]; then
    FIRST="$candidate"
    say "downloading $candidate ($REF)"
  fi
  if download "$BASE/$candidate" "$TMP/$candidate" 2>/dev/null; then
    [ "$candidate" = "$FIRST" ] || say "release $REF has no $FIRST; using $candidate"
    ASSET="$candidate"
    SRC_BIN="${rest%%:*}$EXE"
    NAME="${rest#*:}"
    break
  fi
done
if [ -z "$ASSET" ]; then
  if [ "$REF" = latest ]; then
    fail "no sop release found at https://github.com/$REPO/releases (none published yet?). Build from source instead: cargo install --git https://github.com/$REPO"
  else
    fail "release $REF not found, or it has no $FIRST. See https://github.com/$REPO/releases for available versions"
  fi
fi
BIN="$NAME$EXE"

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
[ -f "$TMP/x/$SRC_BIN" ] || fail "$ASSET doesn't contain $SRC_BIN"

mkdir -p "$INSTALL_DIR"
cp "$TMP/x/$SRC_BIN" "$INSTALL_DIR/$BIN.tmp"
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

say "next: '$NAME skills install' in your repo, then run the $NAME-import skill in Claude Code, Codex or OpenCode"
