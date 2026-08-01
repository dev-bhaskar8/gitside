# Controls

Gitside is fully keyboard-operable. Mouse clicks and the wheel mirror these
actions when terminal mouse events are enabled. In tmux, use `set -g mouse on`.

## Navigation

| Input | Action |
| --- | --- |
| `j` / `k`, arrows | Move selection |
| `Page Up` / `Page Down` | Move ten items |
| `Home` / `End` | First / last item |
| `Tab` / `Shift+Tab` | Next / previous panel |
| `Enter` or double click | Open or activate |
| `[` / `]` | Previous / next repository |
| `/` / `N` | Search focused panel / next result |
| `?` / `F1` | Contextual help |
| `q` / `Ctrl+C` | Quit |

## Changes and commits

| Input | Action |
| --- | --- |
| `Space` | Stage or unstage file or hunk |
| `a` / `u` | Stage / unstage all |
| `d` | Discard with confirmation |
| `e` | External old/new difftool |
| `E` | Interactive line staging |
| `o` | Open working file in configured editor |
| `c` | Focus commit message |
| `Ctrl+G` | Generate an editable commit-message draft when enabled |
| `G` / `Y` | Generate outside the editor / open AI status |
| AI panel: `e`, `1`/`2`/`3`, `x` | Enable; choose Local/Agent/API; toggle emoji |
| `Ctrl+Enter` | Commit |
| `Ctrl+Shift+Enter` | Amend previous commit |
| `Ctrl+Alt+Enter` | Commit with sign-off |
| `Ctrl+Shift+Alt+Enter` | Amend with sign-off |
| `Up` / `Down` in message | Recall commit subjects / restore draft |
| `U` | Undo last commit and retain its changes |

## Remotes and repository

| Input | Action |
| --- | --- |
| `f` / `l` / `p` | Fetch / pull / contextual publish, sync, or push |
| `L` | Pull with rebase |
| `P` / `T` | Push to / pull from a chosen target |
| `F` | Force push with lease |
| `r` | Refresh |
| `D` | Session Git diagnostics |

## Branches, graph, stashes, and worktrees

| Context | Input | Action |
| --- | --- | --- |
| Branches | `n` / `x` | Create / delete branch |
| Branches | `m` / `R` | Merge / rebase selected branch |
| Branches | `w` | Add worktree |
| Graph | `y` / `v` / `t` | Cherry-pick / revert / tag commit |
| Repository | `s` / `z` | Create stash / focus stashes |
| Stashes | `A` / `P` / `X` | Apply / pop / drop stash |
| Repository | `W` | Focus worktrees |
| Worktrees | `X` | Remove selected worktree |

## Conflicts and GitHub

| Context | Input | Action |
| --- | --- | --- |
| Merge Changes | `O` / `I` / `B` | Accept current / incoming / both |
| Git operation | `C` / `S` / `A` | Continue / skip / abort |
| GitHub | `i` | Switch pull requests / issues |
| GitHub | `o` | Open selected item on GitHub |
| Pull request | `C` / `K` | Check out / view checks |

The footer and Help overlay always prioritize controls for the focused panel.
