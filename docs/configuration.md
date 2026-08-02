# Configuration

Gitside loads TOML from the platform configuration directory or the path
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
```

Gitside inherits foreground and background colors from the terminal palette,
so it intentionally has no separate theme configuration.

`refresh_ms` controls polling when native filesystem notifications are
unavailable. With a watcher active, changes are coalesced promptly and a
30-second safety poll protects against dropped platform events.

`confirm_destructive = true` asks before discarding changes, deleting branches,
dropping stashes, removing worktrees, starting a rebase, undoing a commit, or
force-pushing with lease. Setting it to `false` performs those requested actions
immediately.

Editor placeholders are passed as individual process arguments:

- `{path}`: working-tree path

If no editor is configured, Gitside checks `VISUAL`, `EDITOR`, and common
platform editors in that order.

The `e` comparison action uses Git's own difftool configuration. For example:

```sh
git config --global diff.tool vscode
git config --global difftool.vscode.cmd 'code --wait --diff "$LOCAL" "$REMOTE"'
```

## Commit-message generation

Generation is opt-in and always creates an editable draft. It never stages
files, creates a repository, switches branches, commits, or pushes. Every mode
prefers staged changes and falls back to current unstaged and untracked changes
when the index is empty. Enable one of the following modes:

The AI panel is always available in the normal `Tab` sequence, just like
Stashes and Worktrees. From that panel, press `e` to enable generation for the
current session and `1`/`2`/`3` to choose Local/Agent/API. Click Configure or
press `c` to open the guided provider, model, credential,
endpoint, and instruction setup. All controls and wizard choices support both
mouse and keyboard input. Smart Local works immediately, while non-secret
choices are saved automatically across launches.

API keys entered in the wizard are masked and stored in macOS Keychain,
Windows Credential Manager, or the Linux Secret Service. They are never written
to this TOML file. If the platform credential service is unavailable, Gitside
warns that the key is available only for the current session. Environment
variables remain a fallback, and `k` removes the selected provider's stored key.

### Smart Local

This deterministic mode is private, offline, and requires no AI service. It
summarizes the staged index or, when it is empty, the current working tree.

```toml
[ai]
enabled = true
mode = "local"
max_files = 3
max_diff_bytes = 32000
```

### Existing agent

This mode invokes an already authenticated Codex, Claude Code, or OpenCode CLI
non-interactively. The built-in adapters request non-mutating operation and
provide a Git-specific system prompt based on the selected changes. Depending
on the chosen agent, that diff may be sent to its configured provider.

```toml
[ai]
enabled = true
mode = "agent"
instructions = "Use conventional commit subjects."

[ai.agent]
provider = "codex" # codex, claude, opencode, or custom
# model = "your-model"
# args = ["additional", "arguments"]
```

For a custom generator, set `provider = "custom"` and `command` to a trusted
command line. It runs with your user permissions, receives the generation
prompt on standard input, and must print only the proposed commit message to
standard output. For example, Codex can be used with
`codex exec --sandbox read-only -`.

```toml
[ai.agent]
provider = "custom"
command = "/path/to/commit-message-generator"
args = []
```

MCP is not needed for this direction of control: Gitside initiates generation,
so it calls the agent's non-interactive CLI. Agents can continue controlling
repository state through ordinary Git commands.

### Direct API

API keys can be entered through the masked setup wizard or read from environment
variables. They are never stored in the configuration file. This mode sends the
bounded diff to the chosen provider.

```toml
[ai]
enabled = true
mode = "api"
max_diff_bytes = 32000

[ai.api]
provider = "openai" # OpenAI, Anthropic, Gemini, OpenRouter, or Other in the UI
model = "your-model"
# api_key_env = "OPENAI_API_KEY"
```

Default key variables are `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`,
`GEMINI_API_KEY`, and `OPENROUTER_API_KEY`. Choose Other for an OpenAI-compatible
service; it can use a complete chat-completions endpoint and an optional key variable:

```toml
[ai.api]
provider = "compatible"
model = "local-model"
endpoint = "http://127.0.0.1:11434/v1/chat/completions"
# api_key_env = "LOCAL_AI_KEY"
```

Press `Ctrl+G` from any panel, use `G` outside the commit editor, or click
Generate beside Commit. `Y` opens the AI configuration and status panel.
