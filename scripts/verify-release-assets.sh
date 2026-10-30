#!/bin/sh
set -eu

asset_dir=${1:?usage: verify-release-assets.sh ASSET_DIR}
checksums=$asset_dir/sha256.sum
test -f "$checksums"
for artifact in \
  gitside-x86_64-unknown-linux-gnu.tar.xz \
  gitside-aarch64-unknown-linux-gnu.tar.xz \
  gitside-x86_64-pc-windows-msvc.zip \
  gitside-aarch64-pc-windows-msvc.zip; do
  expected=$(awk -v artifact="$artifact" '$2 == artifact || $2 == "*" artifact { print $1 }' "$checksums")
  printf '%s\n' "$expected" | grep -Eq '^[a-fA-F0-9]{64}$' || {
    echo "missing, duplicate, or invalid SHA-256 for $artifact" >&2
    exit 1
  }
  test "$(printf '%s\n' "$expected" | wc -l | tr -d ' ')" = 1
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$asset_dir/$artifact" | awk '{print $1}')
  else
    actual=$(shasum -a 256 "$asset_dir/$artifact" | awk '{print $1}')
  fi
  test "$actual" = "$expected" || {
    echo "checksum mismatch for $artifact" >&2
    exit 1
  }
done
printf 'Canonical release asset checksums verified.\n'
