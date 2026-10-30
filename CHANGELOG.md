# Changelog

All notable changes to Gitside are documented here. Releases follow Semantic
Versioning and use matching `vMAJOR.MINOR.PATCH` Git tags.

## [Unreleased]

## [0.2.0] - 2026-10-31

### Added

- Generate commit drafts with Codex, Claude Code, OpenCode, Cursor, Grok Build,
  Pi, Muse Code, Antigravity, or a compatible installed ACP agent.
- Install the verified Antigravity runtime and sign in with Google inside the
  setup wizard, discover account models, cancel setup, and reconnect expired sessions.
- CLI Antigravity installation and sign-in commands using the same setup backend.
- Provider discovery and a scrolling picker with mouse and keyboard controls.

### Improved

- Shared structured commit prompts, native schema handling where supported,
  validated output, bounded protocol frames, and generation timeouts.
- CLI login reuse, provider-specific model selection, tool isolation, and cleanup
  of owned helper processes. Errors and concurrent edits preserve the user's draft.

### Distribution

- Verify all six hosted native targets before package publication, and call
  downstream package workflows directly from the protected release pipeline.
- Include OpenCode and T3 Code notices in native archives and downstream packages.
- Keep release metadata synchronized and preserve existing WinGet versions when
  preparing a new submission. Registry approvals and store reviews remain required.

## [0.1.2] - 2026-08-02

### Fixed

- Cross-platform release builds now install Linux credential-store headers.
- Windows uses the native TLS backend so ARM cross-builds do not require Rustls' C toolchain.

## [0.1.1] - 2026-08-02

### Added

- Optional Local, Agent, and API commit-message drafts with secure credential setup.
- Responsive staged/unstaged changes, conflict visibility, and contextual help.

### Improved

- Git operations run in the background while the terminal remains interactive.
- Terminal keyboard protocols, paste handling, previews, scrolling, and release packaging.

## [0.1.0] - 2026-08-01

### Added

- Responsive source-control TUI for full terminals and narrow tmux side panes.
- Mouse and keyboard workflows for changes, commits, branches, stashes, worktrees,
  remotes, conflicts, GitHub pull requests, and issues.
- Terminal-native colors, non-blocking repository refresh, and cross-platform
  Git, editor, and GitHub CLI integration.
- Automated multi-platform releases and package-manager distribution.

[Unreleased]: https://github.com/dev-bhaskar8/gitside/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/dev-bhaskar8/gitside/releases/tag/v0.2.0
[0.1.2]: https://github.com/dev-bhaskar8/gitside/releases/tag/v0.1.2
[0.1.1]: https://github.com/dev-bhaskar8/gitside/releases/tag/v0.1.1
[0.1.0]: https://github.com/dev-bhaskar8/gitside/releases/tag/v0.1.0
