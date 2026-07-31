$ErrorActionPreference = "Stop"

$ProjectDirectory = Split-Path -Parent $PSScriptRoot
cargo install --locked --path $ProjectDirectory @args
