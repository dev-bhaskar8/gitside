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

This mode invokes an installed, authenticated coding provider non-interactively.
Gitside sends the supplied Git context through stdin or the provider protocol and extracts the final
message using Codex’s schema-constrained output file, Claude’s structured JSON
result (including verbose message arrays), OpenCode’s JSON text events, ACP text
chunks, Pi’s last assistant text, or Muse’s final message. Progress and reasoning
do not become commit text; protocol helpers reject tool work.
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
provider = "codex" # codex, claude, opencode, antigravity, cursor, grok, pi, muse, acp, custom
# model = "your-model"
# command = "/absolute/path/to/cli" # optional executable override
# args = [] # provider-specific; Pi and Muse utility runs reject launch arguments
```

All eight named providers in [T3 Code's provider list](https://github.com/pingdotgg/t3code/blob/9b1d351ba0f63f527f884ffb3f2e394f50388764/docs/user/install.md) have commit-generation adapters:

| Provider | Executable | Authenticate before generating |
| --- | --- | --- |
| Codex | `codex` | `codex login` |
| Claude Code | `claude` | `claude auth login` |
| OpenCode | `opencode` | `opencode auth login` |
| Antigravity | Managed official Google ACP runtime | Configure → Install Antigravity → Sign in with Google |
| Cursor | `cursor-agent` | `agent login` |
| Grok Build | `grok` | `grok login` |
| Pi | `pi` | Start `pi`, then `/login` |
| Muse Code | `muse` | `muse login` |

Use arrow keys, Page Up/Down, the mouse wheel, or `1`–`9` (`0` for Other) in
Agent setup. The selected row stays visible in small terminals. Installation
checks never claim that an account is authenticated. Install providers on the
machine running Gitside; setup commands run in your normal terminal.

Cursor uses its [official ACP interface](https://cursor.com/docs/cli/acp), rather
than T3's JavaScript SDK, so Gitside needs no Node bridge. It requests native Ask
mode. Grok uses `grok --permission-mode default agent stdio`, authenticates with
its cached login (or `XAI_API_KEY` if present), and supports both normal ACP and
correlated xAI completion notifications. Usage limits are reported as errors.
Both run in empty temporary workspaces, with no client filesystem/terminal
capabilities or supplied MCP servers; tools and interactive requests are rejected.
ACP capability flags do not disable every provider's own global hooks or MCP
configuration and are not an OS sandbox. Use trusted, configured runtimes.

Pi uses `--mode rpc --no-session --no-extensions --no-tools`. An optional model
must use `provider/model` format; extension-only models cannot be used in this
utility run. Pi owns its model-provider authentication, including supported
subscriptions and API keys. Muse uses `serve --disable-shell --disable-write`,
an empty workspace, no requested MSP capabilities, and native `denyUnmatched`
approvals. Gitside follows T3 in ignoring an inherited `META_API_KEY` for Muse,
using its saved login. Muse revisions, retractions, early completion, truncated
answers, and stream gaps are handled before accepting output. Neither helper
accepts extra launch arguments that could re-enable unattended tools. All four
helpers kill their owned Unix process group on timeout and remove the temporary
workspace. Provider-owned global history may still be retained by its runtime.

#### Other ACP agents

An installed ACP v1 stdio agent can use the same bounded text-generation client.
Install and authenticate it using its own instructions, then choose **ACP agent**
and enter its executable. Use TOML for arguments and an optional advertised
non-interactive auth method:

```toml
[ai.agent]
provider = "acp"
command = "/absolute/path/to/agent"
args = ["acp"] # omit when the executable speaks ACP directly
# acp_auth_method = "an-advertised-method-for-existing-credentials"
# model = "an-advertised-model-id"
```

The adapter rejects tools, approvals, and questions, so an agent that requires
those for a text-only request will fail with a clear error. This is compatibility
with installed ACP agents, not T3's registry installer or a claim that every
registry entry has been tested. T3 itself currently disables commit generation
for generic registry agents. Gitside manages Antigravity's runtime and personal
Google sign-in; other providers use their own installers and account setup.
T3's full chat/session orchestration is outside this commit-generation flow.

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

1. Open **Y → Agent → Configure**, choose **Antigravity**, and continue to
   **Connect Antigravity**. First-time Generate also routes here if setup is needed.
2. Select **Install Antigravity**. The managed installer downloads the pinned
   Google runtime for your platform, checks the exact archive size and SHA-256,
   extracts the matching executable/harness pair, validates native ACP startup,
   and activates it atomically. Existing completed installs remain available if
   installation fails. This follows T3 Code's installation strategy and its
   compatible release pin (currently 1.4.1); it does not blindly install the
   registry's newest version. No executable path or PATH change is needed.
3. Choose **Sign in with Google** for an existing runtime. A new installation
   continues directly to sign-in. The browser opens automatically; if it fails,
   choose **Open browser** or open the displayed link manually. Stay in setup
   until Gitside confirms account access. Escape or Back cancels the owned
   setup process; failed sign-in can be retried there.
4. Select an advertised model with Up/Down, type its ID, or leave it blank for
   the account default. Save setup. If setup started from Generate, a successful
   connection saves the choice and resumes generation with the default model.
   The Google runtime retains the login for future requests. Expired sign-in
   returns to this connection step, preserving the current commit draft.

The runtime is stored in the platform local data directory under
`gitside/runtimes/antigravity/versions/<archive-sha256>`, with an atomic active
release record. Only a completed, correctly sized executable/harness pair is
discovered as installed. The credential profile is stored separately.

For terminal setup, `gitside antigravity-install` installs the managed runtime
and `gitside antigravity-login` signs in (installing automatically if needed).
A custom config can be supplied before the command with `--config /path/config.toml`.
CLI sign-in prints the Google URL; the in-app flow opens it automatically.

A manual runtime is still supported. Press **p** or choose **Custom path** in
the connection step, then enter the ACP executable's absolute path. Keep its
matching `localharness_external` helper beside it and make both executable.
An explicit path overrides managed discovery. Press **i** or **Install managed**
to return to Gitside's verified release. Manual installs remain your responsibility.

```toml
[ai]
enabled = true
mode = "agent"

[ai.agent]
provider = "antigravity"
# command = "/absolute/path/to/agy_acp_server.par" # optional manual override
# model = "a-model-id-available-to-your-account"
# antigravity_home = "/optional/private/gitside-antigravity-profile"
```

The default profile is under the platform’s local data directory at
`gitside/antigravity`. Google’s runtime owns credential storage there. Gitside
uses a private profile, removes ambient Google credential overrides from its
child process, and keeps runtime extraction in a temporary directory. Generation
opens a browser only during an explicitly requested connection. An expired login
routes the TUI back to Connect Antigravity; CLI errors still name the login command.

The helper runs in a temporary workspace, advertises no client filesystem or
terminal access, rejects tool/permission/input requests and tool-call updates,
bounds its output, and validates the common commit JSON. It rejects profiles
with global hooks or MCP servers and reports unexpected workspace files.
Processes and owned temporary session files are cleaned up afterward. These
checks do not create a native filesystem sandbox; Gitside does not offer
Antigravity as a general coding agent.

Google's proprietary executable is downloaded from Google on demand and is not
bundled into Gitside. Its release pin follows T3's tested compatibility metadata.
This flow supports personal Google sign-in; enterprise, API-key and Vertex
account modes are not exposed by Gitside. It does not import another app's tokens.

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
