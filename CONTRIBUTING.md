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

Before opening a pull request, run:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
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
