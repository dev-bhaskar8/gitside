# Configuration

Sourcepane loads TOML from the platform configuration directory or the path
passed to `--config`.

```toml
mouse = true
confirm_destructive = true
graph_page_size = 200
refresh_ms = 1500
layout = "auto"

[editor]
command = "code"
args = ["--reuse-window", "--goto", "{path}"]

[theme]
accent = "blue"
added = "green"
deleted = "red"
```

`refresh_ms` controls polling when native filesystem notifications are
unavailable. With a watcher active, changes are coalesced promptly and a
30-second safety poll protects against dropped platform events.

`confirm_destructive = true` asks before discarding changes, deleting branches,
dropping stashes, removing worktrees, or starting a rebase. Setting it to
`false` performs those requested actions immediately.

Editor placeholders are passed as individual process arguments:

- `{path}`: working-tree path

If no editor is configured, Sourcepane checks `VISUAL`, `EDITOR`, and common
platform editors in that order.
