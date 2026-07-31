# Platform support

## macOS

Builds natively on Apple Silicon and Intel Rust targets. Gitside works in
Ghostty, Terminal.app, iTerm2, and tmux. GUI editors such as VS Code can be
configured with their command-line launchers.

## Linux and WSL

Gitside uses the Git and GitHub CLI installed inside the active environment.
A WSL checkout therefore uses WSL paths, Git configuration, SSH agent, and
credentials. Mouse forwarding through tmux requires `set -g mouse on`.

## Native Windows

The application uses `PathBuf` and launches child processes without a shell, so
paths are passed directly to native `git.exe` and `gh.exe`. Windows Terminal
supports the mouse protocol used by Crossterm. Configure a native editor command
such as `code.cmd` when automatic detection does not find one.

## Terminal requirements

The terminal should support an alternate screen, raw keyboard input, ANSI
colors, and SGR mouse reporting for pointer controls. Gitside remains fully
usable from the keyboard when mouse reporting is unavailable or disabled with
`--no-mouse`.
