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

RELEASE_DATE=2026-08-02 scripts/render-package-manifests.sh v0.1.2 "$assets" "$output"
if scripts/render-package-manifests.sh v0.1.2-rc.1 "$assets" "$output/prerelease" >/dev/null 2>&1; then
  echo 'prerelease tag unexpectedly passed the stable-only gate' >&2
  exit 1
fi

if rg -n '__[A-Z0-9_]+__' "$output"; then
  echo 'unrendered package placeholder found' >&2
  exit 1
fi

jq -e '.version == "0.1.2" and .bin == "gitside.exe"' "$output/scoop/gitside.json" >/dev/null
ruby -e 'require "yaml"; ARGV.each { |path| Psych.parse_file(path) }' \
  "$output"/winget/d/DevBhaskar8/Gitside/0.1.2/*.yaml
test -s "$output/chocolatey/gitside.nuspec"
rg -q '<packageSourceUrl>https://github.com/dev-bhaskar8/gitside/tree/main/packaging/chocolatey</packageSourceUrl>' \
  "$output/chocolatey/gitside.nuspec"
rg -q '<iconUrl>https://raw.githubusercontent.com/dev-bhaskar8/gitside/main/site/assets/gitside-mark.svg</iconUrl>' \
  "$output/chocolatey/gitside.nuspec"
! rg -q 'Get-OSArchitectureWidth -eq 32' "$output/chocolatey/tools/chocolateyinstall.ps1"
rg -q '\$nativeArchitecture = \$env:PROCESSOR_ARCHITEW6432' "$output/chocolatey/tools/chocolateyinstall.ps1"
test -s "$output/aur/PKGBUILD"
test -s "$output/aur/.SRCINFO"

aur_src=$work_dir/aur-src
aur_pkg=$work_dir/aur-pkg
mkdir -p "$aur_src/gitside-x86_64-unknown-linux-gnu" "$aur_pkg"
printf '#!/bin/sh\nexit 0\n' > "$aur_src/gitside-x86_64-unknown-linux-gnu/gitside"
cp LICENSE README.md "$aur_src/gitside-x86_64-unknown-linux-gnu/"
(
  CARCH=x86_64
  srcdir=$aur_src
  pkgdir=$aur_pkg
  export CARCH srcdir pkgdir
  # macOS install(1) does not implement GNU install's -D flag used by makepkg.
  # This test double preserves the source/destination contract so the archive
  # layout is exercised on every CI platform.
  # shellcheck disable=SC2329
  install() {
    while [ "$#" -gt 2 ]; do
      shift
    done
    install_source=$1
    install_destination=$2
    mkdir -p "$(dirname "$install_destination")"
    cp "$install_source" "$install_destination"
    chmod 755 "$install_destination"
  }
  # shellcheck disable=SC1090,SC1091
  . "$output/aur/PKGBUILD"
  package
)
test -x "$aur_pkg/usr/bin/gitside"
test -s "$aur_pkg/usr/share/licenses/gitside-bin/LICENSE"
test -s "$aur_pkg/usr/share/doc/gitside-bin/README.md"

printf 'Package templates rendered and validated successfully.\n'
