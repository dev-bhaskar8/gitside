# Gitside

Gitside is a terminal-native source control panel inspired by Visual Studio
Code's Source Control view. It delegates repository operations to your installed
`git` executable and, when available, GitHub workflows to the authenticated
`gh` CLI.

The interface adapts to narrow tmux side panes and full-screen terminals. Mouse
input is supported in Ghostty, tmux, Windows Terminal, and terminals that
implement SGR mouse events; every action also has a keyboard equivalent.

## Status

Gitside is under active development. The first release focuses on a reliable
local Git workflow: repository status, staging, committing, diffs, history,
branches, stashes, remotes, and optional GitHub pull request and issue views.
Working-tree changes are detected through native filesystem notifications,
remote operations run without blocking input, and commit history loads further
pages automatically as the Graph selection approaches the current boundary.

## Requirements

- Rust 1.85 or newer to build
- Git 2.30 or newer
- Optional: GitHub CLI (`gh`) for publishing, pull requests, and issues

Gitside does not store credentials. Git and GitHub CLI continue to use their
existing credential helpers, SSH configuration, signing setup, and keychain.

## Build and run

```sh
cargo build
cargo run -- /path/to/repository
```

Initialize or clone and immediately open a repository:

```sh
gitside init ./new-project
gitside clone https://github.com/owner/project.git
```

Install the current checkout locally:

```sh
cargo install --path .
gitside
```

Or use the local installer for the current platform:

```sh
./scripts/install.sh
```

```powershell
.\scripts\install.ps1
```

Both installers use `cargo install --locked` and accept additional Cargo
arguments such as `--root`. Homebrew, Winget, and downloadable archives require
a public release URL and checksums; those cannot be published while the project
remains intentionally local-only.

Run `gitside --help` for command-line options.

## Controls

| Input | Action |
| --- | --- |
| `j` / `k`, arrows | Move selection |
| `Tab` / `Shift+Tab` | Change panel |
| `Enter` or double click | Open the selected item or terminal diff |
| `Space` | Stage or unstage selected file |
| `a` | Stage all changes |
| `u` | Unstage all staged changes |
| `c` | Focus commit message |
| `Ctrl+Enter` | Commit |
| `Ctrl+Shift+Enter` | Amend the previous commit |
| `Ctrl+Alt+Enter` | Commit with a `Signed-off-by` trailer |
| `Ctrl+Shift+Alt+Enter` | Amend with a `Signed-off-by` trailer |
| `e` | Compare the selected old/new versions using Git's configured difftool |
| `E` | Interactively stage or unstage selected lines |
| `o` | Open the selected working file in the configured editor |
| `f` | Fetch |
| `p` | Push |
| `l` | Pull |
| `L` | Pull with rebase |
| `P` / `T` | Push to / pull from a chosen remote and branch |
| `F` | Confirm and force-push with lease |
| `s` / `z` | Create a stash / open the stash list |
| `r` | Refresh |
| `g` | Focus graph |
| `b` | Focus branches |
| `n` / `x` | Create/delete a branch in the Branches view |
| `m` / `R` | Merge/rebase the selected branch |
| `w` / `W` | Add a worktree from Branches / open the Worktrees view |
| `y` / `v` / `t` | Cherry-pick/revert/tag a selected commit |
| `h` | Focus GitHub |
| `i` / `o` | Switch PR/issues / open the selected item in a browser |
| `C` / `K` | Check out a selected PR / view its checks |
| `O` / `I` / `B` | Resolve a conflicted file with current/incoming/both sides |
| `C` / `A` | Continue/abort an in-progress Git operation |
| `S` | Skip the current rebase, cherry-pick, or revert step |
| `U` | Confirm and undo the last commit while keeping its changes |
| `D` | Show session Git diagnostics |
| `?` | Toggle help |
| `F1` | Toggle help, including while editing a commit message |
| `/` / `N` | Search the focused view / find the next match |
| `q` / `Ctrl+C` | Quit |

While editing a commit message, Up and Down recall earlier commit subjects and
return to the unfinished draft. Undo uses a mixed reset, so committed file
changes remain available in the working tree. Force pushes always use
`--force-with-lease`; Gitside never offers an unguarded force push.

Interactive line staging delegates to Git's patch selector. Use its `s` action
to split a hunk and `e` to edit a patch when a change needs finer selection.
The alternate screen is restored and repository state refreshed afterward.

Single click selects, double click opens, the wheel scrolls, and toolbar buttons
are clickable. Right click opens contextual actions. tmux must have
`set -g mouse on` for mouse events to reach terminal applications.

Fetch, Pull, Push, and Refresh are always available as compact, transparent
one-line outlined buttons. Each button displays its keyboard equivalent; compact
panes reduce them to `[f] [l] [p] [r]` when the full labels cannot fit. The
footer shows shortcuts for the currently focused panel instead of a fixed list
of unexplained keys.

The existing `p` control adapts without adding another button: it becomes
Publish when the current branch has no upstream, Sync when it has both incoming
and outgoing commits, and Push otherwise. Publish prefers `origin`, then falls
back to the first configured remote. When no remote exists and authenticated
GitHub CLI support is available, the same control opens the confirmed GitHub
repository publishing flow.

Help opens with shortcuts for the currently focused panel, followed by the
complete reference. Use `j`/`k`, arrows, Page Up/Down, Home/End, or the mouse
wheel to scroll; the border shows the current position and remaining overflow.

Search is invoked on demand with `/` and disappears after submission or
cancellation. It searches only the focused Changes, Staged, Graph, Branches,
Stashes, Worktrees, GitHub, or Preview view; `N` advances to the next match.

Gitside rechecks the installed GitHub CLI, its authentication, and repository
remotes during normal repository refreshes. After `gh auth login`, no restart is
needed. If an authenticated repository has no GitHub remote, the GitHub panel
shows a contextual Publish action. Publishing asks for an editable repository
name, private or public visibility (private is selected first), and a final
confirmation. Gitside never creates a remote repository automatically.

When conflicts exist, the normal Changes panel temporarily becomes Merge
Changes and shows only conflicted files. Resolution and continue/abort actions
exist only in that repository state and disappear after the operation ends.

## Configuration

The default configuration file is:

- macOS: `~/Library/Application Support/gitside/config.toml`
- Linux/WSL: `$XDG_CONFIG_HOME/gitside/config.toml`
- Windows: `%APPDATA%\gitside\config.toml`

See [Configuration](docs/configuration.md) and
[Architecture](docs/architecture.md). Platform-specific behavior is documented
in [Platform support](docs/platforms.md).

## Project layout

- `src/git.rs`: safe Git command adapter and parsers
- `src/github.rs`: optional GitHub CLI adapter
- `src/app.rs`: application state and commands
- `src/ui.rs`: responsive rendering and mouse hit regions
- `src/config.rs`: CLI and TOML configuration
- `scripts/`: local macOS/Linux and Windows installers
- `docs/`: extended documentation and troubleshooting

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The repository intentionally has no configured remote. Publishing or linking it
to a hosting provider is outside the local development workflow.

## License

Gitside is available under the MIT License.
