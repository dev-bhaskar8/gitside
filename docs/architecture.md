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

Repository mutations, previews, fetch, pull, push, refreshes, GitHub requests,
and history pagination run as background tasks so the event and rendering loop
remains responsive. Snapshot reads run concurrently inside their task. A single
task slot serializes repository work and prevents overlapping mutations.
Completed actions refresh the repository they started from, even if the user
has switched views.

Commit-message generation uses that same background-task boundary and inserts
only an editable draft. The `ai` adapter supports deterministic local rules,
built-in agent adapters that request non-mutating operation, trusted custom
commands, and direct HTTPS APIs. All modes prefer the staged index and use a
read-only working-tree fallback when it is empty; direct API input is
byte-bounded, credentials come from the OS credential store or an
environment fallback, and failures preserve the user's current draft.

The first Generate action opens an installed-CLI picker. Choosing a built-in
CLI saves the provider and continues generation with its own model and login.
Detection checks executability without reading credentials or calling a model.
Configuration remains available for optional overrides and explicit offline/API
modes. Switching providers drops incompatible executable/argument overrides.

The adapter drains stdout/stderr concurrently with stdin writes, closes stdin
at EOF, and applies a configurable total deadline. Codex final-message files,
Claude JSON results, and OpenCode JSON text events are normalized into the same
editable draft result. A snapshot of the starting draft prevents a late result
from overwriting edits. Provider errors remain errors rather than draft text.

Generation follows OpenCode's standalone utility strategy: build a system
prompt and user input, resolve the chosen provider, make one text-generation
request, and return its text. `src/ai.rs` owns Git context and draft cleanup;
`src/ai/provider.rs` owns the shared `TextRequest` and provider transports.
Repository data stays in user input; configured instructions stay in the
system prompt.

Like OpenCode's utility calls, Claude gets a replacement system prompt and no
built-in tools, hooks, memory, or external MCP configuration. Codex gets a
temporary replacement instructions file, disables shell tools and repository
instruction loading, and retains its read-only sandbox. OpenCode gets a
process-local `gitside-commit` agent with the task prompt, all tool permissions
denied, and one step. Its inherited inline JSON/JSONC provider and model
settings are preserved. Custom generators receive the combined prompt on stdin.
CLI authentication and model defaults stay with the CLI.

The Rust adaptation follows OpenCode's
[Generate.text boundary](https://github.com/anomalyco/opencode/blob/bf5d12c588911a35ec56ad991c3b83cf937f424a/packages/core/src/generate.ts),
[standalone Claude request](https://github.com/anomalyco/opencode/blob/bf5d12c588911a35ec56ad991c3b83cf937f424a/packages/core/src/plugin/provider/claude-agent/language-model/claude-agent-language-model.ts),
[utility system prompt](https://github.com/anomalyco/opencode/blob/bf5d12c588911a35ec56ad991c3b83cf937f424a/packages/core/src/plugin/provider/claude-agent/language-model/system-prompt.ts),
and [isolation options](https://github.com/anomalyco/opencode/blob/bf5d12c588911a35ec56ad991c3b83cf937f424a/packages/core/src/plugin/provider/claude-agent/claude/query-options.ts).
It uses native CLI interfaces without requiring a Node/Agent SDK runtime.
See the [third-party notice](third-party-notices.md).

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

Conflict handling uses repository state as progressive disclosure. Conflicted
rows are marked `C` alongside ordinary staged and unstaged changes; selecting
one exposes whole-file current/incoming/both plus operation continue/abort
commands through contextual shortcuts and Help. Resolving a file stages it so
Git can recognize that the conflict is complete.

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
