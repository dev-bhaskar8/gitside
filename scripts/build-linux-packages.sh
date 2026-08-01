#!/bin/sh
set -eu

tag=${1:?usage: build-linux-packages.sh TAG ASSET_DIR OUTPUT_DIR}
asset_dir=${2:?usage: build-linux-packages.sh TAG ASSET_DIR OUTPUT_DIR}
output_dir=${3:?usage: build-linux-packages.sh TAG ASSET_DIR OUTPUT_DIR}

printf '%s\n' "$tag" | grep -Eq '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' || {
  echo "release tag must be a stable vMAJOR.MINOR.PATCH" >&2
  exit 1
}

command -v nfpm >/dev/null 2>&1 || {
  echo "nfpm is required" >&2
  exit 1
}

version=${tag#v}
mkdir -p "$output_dir"
work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT HUP INT TERM

build_arch() {
  rust_arch=$1
  deb_arch=$2
  rpm_arch=$3
  archive=$asset_dir/gitside-$rust_arch-unknown-linux-gnu.tar.xz
  extract_dir=$work_dir/$rust_arch

  test -f "$archive"
  mkdir -p "$extract_dir"
  tar -xJf "$archive" -C "$extract_dir"
  binary=$(find "$extract_dir" -type f -name gitside -perm -111 | head -n 1)
  test -n "$binary"

  deb_config=$work_dir/nfpm-$deb_arch.yaml
  rpm_config=$work_dir/nfpm-$rpm_arch.yaml
  sed -e "s|__VERSION__|$version|g" -e "s|__BINARY__|$binary|g" \
    -e "s|__NFPM_ARCH__|$deb_arch|g" packaging/nfpm.yaml > "$deb_config"
  sed -e "s|__VERSION__|$version|g" -e "s|__BINARY__|$binary|g" \
    -e "s|__NFPM_ARCH__|$rpm_arch|g" packaging/nfpm.yaml > "$rpm_config"

  nfpm package --config "$deb_config" --packager deb \
      --target "$output_dir/gitside_${version}_${deb_arch}.deb"
  nfpm package --config "$rpm_config" --packager rpm \
      --target "$output_dir/gitside-${version}-1.${rpm_arch}.rpm"
}

build_arch x86_64 amd64 x86_64
build_arch aarch64 arm64 aarch64
