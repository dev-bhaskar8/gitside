# Architecture

Gitside is a presentation and orchestration layer over the command-line tools
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
are inherited by child processes and never copied into Gitside state or logs.

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

Fetch, pull, push, repository refreshes, GitHub list loading, and history
pagination run as background tasks so the event and rendering loop remains
responsive. Snapshot reads run concurrently inside their task. A single task
slot serializes repository work and prevents overlapping network mutations.
Completed remote actions refresh the repository they started from, even if the
user has switched views.

Commit-message generation uses that same background-task boundary and inserts
only an editable draft. The `ai` adapter supports deterministic local rules,
built-in agent adapters that request non-mutating operation, trusted custom
commands, and direct HTTPS APIs. All modes inspect only the staged index; direct
API input is byte-bounded, credentials come from the OS credential store or an
environment fallback, and failures preserve the user's current draft.

The AI setup wizard persists only non-secret settings in the TOML document and
preserves unrelated configuration. API keys use the native OS credential store
and are redacted from overlays, diagnostics, errors, and serialization. When a
credential store is unavailable, the secret is held only in zeroizing process
memory for the current session. Credential operations run on blocking workers
so keychain access never executes in the renderer.

Every repository snapshot also classifies GitHub integration as CLI missing,
unauthenticated, authenticated without a GitHub remote, or ready. This makes a
new `gh` installation or login visible through the existing refresh cycle. The
no-remote state progressively exposes a publish flow; repository name and
visibility live in temporary overlays, and the external create operation always
requires final confirmation before it runs as a background task.

Native filesystem notifications mark repository state dirty and are coalesced
before refresh. Git object-store churn and read-only events are ignored. A
low-frequency safety poll remains active, and the configured polling interval is
used as the fallback on platforms where a watcher cannot be created.

Conflict handling uses repository state as progressive disclosure. The Changes
panel switches to Merge Changes only while conflicts exist, and whole-file
current/incoming/both plus operation continue/abort commands are then exposed
through contextual shortcuts and Help. Resolving a file stages it so Git can
recognize that the conflict is complete.

Search is an ephemeral overlay rather than a persistent toolbar. A query maps
to searchable text owned by the focused view, selects the next matching model
index, and is retained only so `N` can continue the search after the overlay
closes.

Advanced operations follow the same zero-clutter rule. Remote target selection,
force-with-lease confirmation, undo confirmation, and diagnostics appear only
when invoked. External comparisons and line staging temporarily leave the
alternate screen and delegate to Git's configured difftool and patch selector,
then restore the terminal and refresh the repository snapshot.

Remote actions also use progressive disclosure. The existing push hit region
selects publish, sync, or push from branch upstream/ahead/behind state, and its
label changes in place. If publishing has no target remote but authenticated
GitHub CLI support is available, that same action enters the repository publish
flow. This adds the missing workflow without increasing the toolbar's control
count.
