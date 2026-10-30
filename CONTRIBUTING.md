# Contributing to Gitside

Thanks for helping make terminal source control better.

## Before you start

- Search existing issues before opening a duplicate.
- Keep changes focused and preserve keyboard-only operation.
- Test narrow side panes as well as ordinary full-screen layouts.
- Do not add a permanent control when a contextual action can expose the same workflow.

## Local development

```sh
git clone https://github.com/dev-bhaskar8/gitside.git
cd gitside
cargo build
cargo run -- /path/to/a/test/repository
```

Linux source builds need the D-Bus development headers used by secure API-key
storage. On Debian/Ubuntu, install `pkg-config libdbus-1-dev`; on Fedora, install
`pkgconf-pkg-config dbus-devel`. Git and Python 3 are needed by integration tests.

Before opening a pull request, run:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
scripts/check-release-metadata.sh
scripts/test-packaging.sh
scripts/check-release-workflow.sh
```

Pull requests should explain the user-visible problem, the chosen interaction,
and how both narrow and wide layouts were verified. Include screenshots for UI
changes when possible.

## Design principles

1. Preserve a zero-clutter default interface.
2. Keep every mouse action reachable from the keyboard.
3. Inherit terminal colors and remain legible in light and dark themes.
4. Never invoke repository-provided text through a shell.
5. Confirm destructive or externally visible operations.

By contributing, you agree that your contribution is licensed under the MIT
License included with this repository.

## Packaging and releases

Read [Packages and releases](docs/distribution.md) before preparing a release.
Every native archive must carry the third-party notices, and all package versions
must match Cargo.toml. Release builds verify six target binaries before protected
registry publication. Store reviews are tracked separately from successful builds.
