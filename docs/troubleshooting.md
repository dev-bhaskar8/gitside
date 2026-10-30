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
