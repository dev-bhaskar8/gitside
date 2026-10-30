#!/bin/sh
set -eu

version=$(cargo metadata --locked --no-deps --format-version=1 | jq -r '.packages[] | select(.name == "gitside") | .version')
snap_version=$(sed -n "s/^version: '\([^']*\)'$/\1/p" snap/snapcraft.yaml)
test "$version" = "$snap_version" || {
  echo "Cargo version $version does not match Snap version $snap_version" >&2
  exit 1
}
grep -Fq "## [$version] -" CHANGELOG.md || {
  echo "CHANGELOG.md is missing release notes for $version" >&2
  exit 1
}
printf 'Release metadata is synchronized at %s.\n' "$version"
