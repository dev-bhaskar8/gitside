# Troubleshooting

## Mouse input in tmux

Enable mouse forwarding with `set -g mouse on`. Keyboard controls remain
available when mouse reporting is disabled.

## Terminal appears corrupted after interruption

Sourcepane restores the alternate screen, cursor, raw mode, and mouse capture on
normal exit and handled errors. If the process is killed ungracefully, run
`reset`.

## GitHub view unavailable

Install GitHub CLI and run `gh auth login`. Local Git features do not require
GitHub CLI.

## An action needs credentials or signing

Git uses the same credential helpers, SSH agent, and pinentry configuration as
the command line. Sourcepane records failed-action diagnostics for the session;
press `D` to display them on demand.
