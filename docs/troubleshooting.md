# Troubleshooting

## Mouse input in tmux

Enable mouse forwarding with `set -g mouse on`. Keyboard controls remain
available when mouse reporting is disabled.

## Terminal appears corrupted after interruption

Gitside restores the alternate screen, cursor, raw mode, and mouse capture on
normal exit and handled errors. If the process is killed ungracefully, run
`reset`.

## GitHub view unavailable

Install GitHub CLI and run `gh auth login`. Local Git features do not require
GitHub CLI.

## An action needs credentials or signing

Git uses the same credential helpers, SSH agent, and pinentry configuration as
the command line. Gitside records failed-action diagnostics for the session;
press `D` to display them on demand.

## Commit generation

- **Installed, but generation fails:** installation detection does not establish
  authentication. Sign in with the selected CLI and try again. Errors preserve
  the current commit draft.
- **Invalid commit JSON:** built-in providers must return string `subject` and
  `body` fields. Check that your custom model supports the selected provider’s
  structured-output option. Custom generator commands may still return plain text.
- **Codex MCP inspection fails:** update the CLI and fix its configuration so
  `codex mcp list --json` works. Gitside stops rather than launching generation
  with unknown MCP settings.
- **Cursor missing:** install Cursor CLI, which provides `cursor-agent`, and run
  `agent login`. If your installation uses another path, set `ai.agent.command`.
  The adapter requires a CLI version supporting ACP and native Ask mode.
- **Grok login or usage limits:** run `grok login`; an inherited `XAI_API_KEY`
  selects Grok's API-key auth instead. Limit errors preserve your draft.
- **Pi model rejected:** use `provider/model`, or clear the override. Utility
  generation disables extensions and cannot use extension-only models. Run
  `pi`, then `/login`, to check the selected account/provider. Update Pi if its
  RPC server does not report `agent_settled` or `get_last_assistant_text`.
- **Muse unavailable or signed out:** install Muse Code and run `muse login`.
  Text generation needs MSP schema v1 and uses the saved login; inherited
  `META_API_KEY` is ignored. Clear an unavailable model override.
- **Pi/Muse launch arguments rejected:** clear `ai.agent.args` and use
  `ai.agent.model`; these utility runs enforce their own read-only launch flags.
- **ACP requests tools or input:** the utility client rejects these requests.
  Use a text-only compatible runtime or another provider. If authentication is
  needed, sign in using the runtime's own setup before generation.
- **A provider is below the visible picker rows:** use arrows, Page Up/Down,
  or the mouse wheel; `1`–`9` select entries and `0` selects Other.
- **Antigravity runtime missing:** `agy` is a different executable. Install the
  official ACP archive and configure `agy_acp_server.par` (Windows: `.exe`), with
  its matching harness alongside it.
- **Antigravity needs sign-in:** run `gitside antigravity-login` after saving the
  runtime path in AI setup. This login is separate from the IDE/CLI. For a custom
  configuration file, pass `--config` before the login subcommand.
- **Antigravity model unavailable:** clear the model override to use the account’s
  default or select a model ID available to that Google account.
- **Generation times out:** check the provider connection or increase
  `ai.timeout_secs` (default 120). Antigravity login has a separate five-minute
  deadline.

For installation and profile details, see
[Antigravity configuration](configuration.md#google-antigravity).
