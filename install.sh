#!/bin/sh
# Installs the newest release of `best` for this machine: the archive is taken from GitHub, checked against the
# release's SHA256SUMS, and the binary put in ~/.local/bin (BEST_INSTALL_DIR chooses another place).
#
#   curl -fsSL https://raw.githubusercontent.com/behavioralstate/best-cli/main/install.sh | sh
#
# BEST_VERSION=0.1.0 pins a release instead of the newest.
set -eu

repo="behavioralstate/best-cli"
dir="${BEST_INSTALL_DIR:-$HOME/.local/bin}"

os=$(uname -s)
arch=$(uname -m)
case "$os" in
  Linux) libc=unknown-linux-musl ;;
  Darwin) libc=apple-darwin ;;
  *) echo "best: no release for $os; build it with: cargo install --git https://github.com/$repo" >&2; exit 1 ;;
esac
case "$arch" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  *) echo "best: no release for $arch; build it with: cargo install --git https://github.com/$repo" >&2; exit 1 ;;
esac
if [ "$os" = Darwin ] && [ "$arch" = x86_64 ]; then
  echo "best: no release for Intel macOS yet; build it with: cargo install --git https://github.com/$repo" >&2
  exit 1
fi
target="$arch-$libc"

if [ -n "${BEST_VERSION:-}" ]; then
  base="https://github.com/$repo/releases/download/v$BEST_VERSION"
  archive="best-$BEST_VERSION-$target.tar.gz"
else
  base="https://github.com/$repo/releases/latest/download"
  archive="best-$target.tar.gz"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "Downloading $base/$archive"
curl -fsSL -o "$tmp/$archive" "$base/$archive"
curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"
cd "$tmp"
expected=$(grep " $archive\$" SHA256SUMS | awk '{print $1}')
[ -n "$expected" ] || { echo "best: $archive is not in SHA256SUMS" >&2; exit 1; }
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$archive" | awk '{print $1}')
else
  actual=$(shasum -a 256 "$archive" | awk '{print $1}')
fi
[ "$expected" = "$actual" ] || { echo "best: the checksum of $archive does not match SHA256SUMS" >&2; exit 1; }
tar xzf "$archive"
mkdir -p "$dir"
install -m 0755 "best-$target/best" "$dir/best"
echo "Installed $("$dir/best" --version) in $dir"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "Add $dir to your PATH, then: best login <name> --manifest <address>" ;;
esac
