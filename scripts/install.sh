#!/bin/sh
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(dirname -- "$script_dir")

exec cargo install --locked --path "$project_dir" "$@"
