#!/bin/sh
set -eu

tag=${1:?usage: render-package-manifests.sh TAG ASSET_DIR OUTPUT_DIR}
asset_dir=${2:?usage: render-package-manifests.sh TAG ASSET_DIR OUTPUT_DIR}
output_dir=${3:?usage: render-package-manifests.sh TAG ASSET_DIR OUTPUT_DIR}

printf '%s\n' "$tag" | grep -Eq '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' || {
  echo "release tag must be a stable vMAJOR.MINOR.PATCH" >&2
  exit 1
}

version=${tag#v}
checksums=$asset_dir/sha256.sum
test -f "$checksums"

checksum() {
  artifact=$1
  value=$(awk -v artifact="$artifact" '$2 == artifact || $2 == "*" artifact { print $1 }' "$checksums")
  test -n "$value" || {
    echo "missing checksum for $artifact" >&2
    exit 1
  }
  printf '%s' "$value"
}

x64_zip=$(checksum gitside-x86_64-pc-windows-msvc.zip)
arm64_zip=$(checksum gitside-aarch64-pc-windows-msvc.zip)
x64_linux=$(checksum gitside-x86_64-unknown-linux-gnu.tar.xz)
arm64_linux=$(checksum gitside-aarch64-unknown-linux-gnu.tar.xz)
release_date=${RELEASE_DATE:-$(date -u +%Y-%m-%d)}

render() {
  source_file=$1
  destination=$2
  mkdir -p "$(dirname "$destination")"
  sed \
    -e "s/__VERSION__/$version/g" \
    -e "s/__RELEASE_DATE__/$release_date/g" \
    -e "s/__X64_ZIP_SHA256__/$x64_zip/g" \
    -e "s/__ARM64_ZIP_SHA256__/$arm64_zip/g" \
    -e "s/__X64_LINUX_SHA256__/$x64_linux/g" \
    -e "s/__ARM64_LINUX_SHA256__/$arm64_linux/g" \
    "$source_file" > "$destination"
}

render packaging/scoop/gitside.json.tmpl "$output_dir/scoop/gitside.json"
render packaging/chocolatey/gitside.nuspec.tmpl "$output_dir/chocolatey/gitside.nuspec"
render packaging/chocolatey/tools/chocolateyinstall.ps1.tmpl "$output_dir/chocolatey/tools/chocolateyinstall.ps1"
render packaging/aur/PKGBUILD.tmpl "$output_dir/aur/PKGBUILD"
render packaging/aur/.SRCINFO.tmpl "$output_dir/aur/.SRCINFO"

winget_dir=$output_dir/winget/d/DevBhaskar8/Gitside/$version
render packaging/winget/DevBhaskar8.Gitside.installer.yaml.tmpl "$winget_dir/DevBhaskar8.Gitside.installer.yaml"
render packaging/winget/DevBhaskar8.Gitside.locale.en-US.yaml.tmpl "$winget_dir/DevBhaskar8.Gitside.locale.en-US.yaml"
render packaging/winget/DevBhaskar8.Gitside.yaml.tmpl "$winget_dir/DevBhaskar8.Gitside.yaml"

printf 'Rendered package-manager manifests for %s in %s\n' "$tag" "$output_dir"
