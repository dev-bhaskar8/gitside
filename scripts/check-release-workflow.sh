#!/bin/sh
set -eu

saved_workflow=$(mktemp)
saved_config=$(mktemp)
cp .github/workflows/release.yml "$saved_workflow"
cp dist-workspace.toml "$saved_config"

restore() {
  cp "$saved_workflow" .github/workflows/release.yml
  cp "$saved_config" dist-workspace.toml
  rm -f "$saved_workflow" "$saved_config"
}
trap restore EXIT HUP INT TERM

# cargo-dist refuses to regenerate a file explicitly marked allow-dirty. Remove
# that local exception only for this isolated reproducibility check.
# shellcheck disable=SC2016 # Ruby's $_ is intentionally quoted for the shell.
ruby -pi -e '$_ = "" if $_ == "allow-dirty = [\"ci\"]\n"' dist-workspace.toml
rm .github/workflows/release.yml
dist generate --mode=ci
scripts/patch-release-workflow.sh
cmp "$saved_workflow" .github/workflows/release.yml

printf 'Release workflow matches cargo-dist plus the deterministic patch.\n'
