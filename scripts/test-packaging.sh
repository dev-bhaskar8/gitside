#!/bin/sh
set -eu

work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT HUP INT TERM
assets=$work_dir/assets
output=$work_dir/output
mkdir -p "$assets"

artifacts='gitside-x86_64-pc-windows-msvc.zip
gitside-aarch64-pc-windows-msvc.zip
gitside-x86_64-unknown-linux-gnu.tar.xz
gitside-aarch64-unknown-linux-gnu.tar.xz'

: > "$assets/sha256.sum"
index=1
printf '%s\n' "$artifacts" | while IFS= read -r artifact; do
  checksum=$(printf '%064d' "$index")
  printf '%s  %s\n' "$checksum" "$artifact" >> "$assets/sha256.sum"
  index=$((index + 1))
done

RELEASE_DATE=2026-08-01 scripts/render-package-manifests.sh v0.1.0 "$assets" "$output"
if scripts/render-package-manifests.sh v0.1.0-rc.1 "$assets" "$output/prerelease" >/dev/null 2>&1; then
  echo 'prerelease tag unexpectedly passed the stable-only gate' >&2
  exit 1
fi

if rg -n '__[A-Z0-9_]+__' "$output"; then
  echo 'unrendered package placeholder found' >&2
  exit 1
fi

jq -e '.version == "0.1.0" and .bin == "gitside.exe"' "$output/scoop/gitside.json" >/dev/null
ruby -e 'require "yaml"; ARGV.each { |path| Psych.parse_file(path) }' \
  "$output"/winget/d/DevBhaskar8/Gitside/0.1.0/*.yaml
test -s "$output/chocolatey/gitside.nuspec"
test -s "$output/aur/PKGBUILD"
test -s "$output/aur/.SRCINFO"

printf 'Package templates rendered and validated successfully.\n'
