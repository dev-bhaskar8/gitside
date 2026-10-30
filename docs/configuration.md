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
when the index is empty.

On first Generate (`Ctrl+G`), choose a coding CLI from the picker. Installed
CLIs are labelled; installation detection does not verify authentication.
Press Enter to save your selection and generate with its existing login and
model defaults. There is no Gitside API-key setup for this route. If no supported
CLI is installed, install and sign in to one, reopen the picker, or use `Y` to
explicitly select offline rules or a direct API.

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
non-interactively. Gitside supplies the diff on stdin and extracts the final
message using Codex’s schema-constrained output file, Claude’s structured JSON
result (including verbose message arrays), or OpenCode’s JSON text events. Progress, reasoning, and tool events do not become commit text.
The model is optional; leaving it blank keeps the CLI's own default.

Generation ports T3 Code’s shared commit prompt and `{subject, body}` output
contract. Every built-in AI provider must return valid structured output before
Gitside creates a draft. Subjects are reduced to one line, lose trailing periods,
and are capped at 72 characters without splitting a grapheme. Malformed, empty,
or oversized replies leave your draft intact.

Claude uses its native JSON-schema option with tools, hooks, and external MCP
configuration disabled. Codex uses its native output-schema option, read-only
sandbox, and a temporary instructions file. Apps/plugins and named MCP servers
are disabled for that request; a failed MCP inspection stops generation.
OpenCode uses a process-local tool-denied commit agent. Overrides apply only
to launched processes; the CLI logins remain available.

```toml
[ai]
enabled = true
mode = "agent"
instructions = "Use conventional commit subjects."
timeout_secs = 120

[ai.agent]
provider = "codex" # codex, claude, opencode, antigravity, or custom
# model = "your-model"
# args = ["additional", "arguments"]
```

For a custom generator, set `provider = "custom"` and `command` to a trusted
command line. It runs with your user permissions, receives the generation
prompt on standard input, and may print either a plain commit message or the shared JSON object to
standard output. For example, Codex can be used with
`codex exec --sandbox read-only -`.

```toml
[ai.agent]
provider = "custom"
command = "/path/to/commit-message-generator"
args = []
```

Generation has a total deadline (`ai.timeout_secs`, default 120 seconds),
and the directly launched process is killed if the request times out. Errors
preserve the current draft. Successful output is discarded if you changed the
draft or switched repositories while waiting. Provider switching clears the
previous CLI's custom executable and extra arguments.

MCP is not needed for this direction of control: Gitside initiates generation,
so it calls the agent's non-interactive CLI. Agents can continue controlling
repository state through ordinary Git commands.

### Google Antigravity

Antigravity uses Google’s official **ACP runtime**, as in T3 Code. Its sign-in is
separate from the Antigravity IDE and `agy` CLI; Gitside does not copy their tokens.
Gitside supplies an ACP v1 client using its own client identity.

1. Download your platform’s archive from the
   [official ACP Registry entry](https://github.com/agentclientprotocol/registry/blob/main/antigravity-acp/agent.json).
   Verify its SHA-256 using the registry value and extract the complete archive;
   keep `agy_acp_server.par` (Windows: `agy_acp_server.exe`) and the
   `localharness_external` helper together. Make both files executable on macOS
   and Linux.
2. In `Y` → Agent → Configure, choose **Antigravity**, enter the ACP executable’s
   absolute path, and save. You can also place the executable on `PATH`.
3. From a normal terminal, run `gitside antigravity-login`. Open the Google link
   printed by that command and wait for the success message. If you use a custom
   config file, run `gitside --config /path/config.toml antigravity-login`.
4. Generate a commit draft. Leave the model blank for the runtime default, or
   enter a model ID advertised to Gitside by the runtime. Availability depends on
   Google’s account and client eligibility. Gitside reports unavailable
   selections instead of silently switching models.

```toml
[ai]
enabled = true
mode = "agent"

[ai.agent]
provider = "antigravity"
command = "/absolute/path/to/agy_acp_server.par"
# model = "a-model-id-available-to-your-account"
# antigravity_home = "/optional/private/gitside-antigravity-profile"
```

The default profile is under the platform’s local data directory at
`gitside/antigravity`. Google’s runtime owns credential storage there. Gitside
uses a private profile, removes ambient Google credential overrides from its
child process, and keeps runtime extraction in a temporary directory. Generation
does not open a login browser; an expired login asks you to run the login command.

The helper runs in a temporary workspace, advertises no client filesystem or
terminal access, rejects tool/permission/input requests and tool-call updates,
bounds its output, and validates the common commit JSON. It rejects profiles
with global hooks or MCP servers and reports unexpected workspace files.
Processes and owned temporary session files are cleaned up afterward. These
checks do not create a native filesystem sandbox; Gitside does not offer
Antigravity as a general coding agent.

The runtime must be installed separately. Gitside neither bundles nor updates
Google’s proprietary binaries. This integration supports Google personal-account
sign-in; enterprise/Vertex setup and managed runtime installation are outside
this commit-drafting flow.

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
