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

Editor placeholders are passed as individual process arguments:

- `{path}`: working-tree path
- `{old}`: temporary path containing the old side of a diff
- `{new}`: working-tree path

If no editor is configured, Sourcepane checks Git configuration, `VISUAL`,
`EDITOR`, and common platform editors in that order.

