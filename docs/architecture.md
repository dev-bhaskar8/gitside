# Architecture

Sourcepane is a presentation and orchestration layer over the command-line tools
developers already trust. It never implements a competing Git object database.

## Data flow

1. The CLI resolves one or more repository paths.
2. `GitRepo` invokes Git without a shell and parses stable machine-readable
   formats such as porcelain v2 with NUL separators.
3. `App` owns a snapshot of status, history, refs, remotes, and optional GitHub
   data.
4. Input events become typed actions. Mutating actions run through the adapter
   and refresh the snapshot afterward.
5. `ui` renders the snapshot for the current terminal dimensions and records
   mouse hit regions for the next event.

No repository-provided string is interpolated into a shell command. Credentials
are inherited by child processes and never copied into Sourcepane state or logs.

## Responsive model

- Under 100 columns, Changes and Graph remain vertically stacked for tall side
  panes, including panes narrower than 58 columns.
- When fewer than 20 body rows are available, one focused view occupies the
  body so actions do not become unusably compressed.
- From 100 through 159 columns, navigation and detail are shown side by side.
- At 160 columns and above, changes, history, and detail can be visible together.

The renderer recalculates geometry and hit regions for every frame, including
tmux pane resize events.

The header uses compact one-line outlined remote-action controls with their
keyboard equivalents embedded in the labels. At narrow widths the toolbar moves
below the repository title and reduces to `[f] [l] [p] [r]`. Footer hints are
derived from the focused panel and status text is truncated by terminal-cell
width rather than UTF-8 byte length.

The help overlay derives its first section from the active focus target and
then presents the full shortcut reference. It owns an independent, clamped
scroll position and renders both a textual overflow indicator and a minimal
offset-based scrollbar. The thumb maps `scroll / max_scroll` directly onto the
track so it reaches the first and last cells exactly without suggesting that
unavailable directions are active.

Advanced commit modes remain keyboard-first instead of adding another menu.
Modifier combinations map to explicit commit options, and Commit-focused Help
lists the full set before the general reference. `F1` opens Help from the commit
editor so `?` remains available as ordinary message punctuation.

## Process model

Read commands capture stdout and stderr. Actions that may require an interactive
editor, credential prompt, or pinentry temporarily leave the alternate screen
and inherit the terminal. Git and `gh` retain responsibility for authentication,
hooks, signing, SSH, LFS, and transport behavior.
