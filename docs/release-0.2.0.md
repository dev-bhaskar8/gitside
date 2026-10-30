# Gitside 0.2.0 release candidate

This record prepares the next release. It does not claim publication. The public
release was still 0.1.2 when checked on October 31, 2026.

## Scope

- Eight named coding providers plus compatible ACP agents for Generate with AI.
- Managed, verified Antigravity runtime installation and personal Google sign-in
  inside setup, account model discovery, cancellation and expired-session recovery.
- Structured commit validation, existing CLI login reuse, bounded context/output,
  process cleanup and preservation of drafts edited during generation.
- Updated README, website, source/package metadata, license notices, and release
  workflow connections. The historical 0.1.2 benchmark remains explicitly dated.

## Distribution audit

| Channel | Evidence | Required action |
| --- | --- | --- |
| GitHub/native archives | [Latest hosted release](https://github.com/dev-bhaskar8/gitside/releases/tag/v0.1.2) | Build and host 0.2.0 after tag authorization and release approval |
| Cargo | [Registry entry](https://crates.io/crates/gitside) | Publish 0.2.0 through the protected publisher |
| npm | [Package](https://www.npmjs.com/package/gitside) | Publish the generated 0.2.0 installer package |
| Homebrew | [Tap formula](https://github.com/dev-bhaskar8/homebrew-tap/blob/main/Formula/gitside.rb) | Update formula from verified 0.2.0 artifacts |
| Scoop | [Bucket manifest](https://github.com/dev-bhaskar8/scoop-bucket/blob/main/bucket/gitside.json) | Update both Windows architectures |
| AUR | [Package](https://aur.archlinux.org/packages/gitside-bin), [failed update](https://github.com/dev-bhaskar8/gitside/actions/runs/30767207185) | Retry; the logged failure was AUR maintenance, not proof of a bad key |
| Chocolatey | [Failed package push](https://github.com/dev-bhaskar8/gitside/actions/runs/30763992018) | Resolve HTTP 403 and confirm ownership/moderation before publishing |
| Snap | [Blocked upload](https://github.com/dev-bhaskar8/gitside/actions/runs/30766827069) | Resolve the earlier pending review in the store dashboard |
| WinGet | [Closed 0.1.2 submission](https://github.com/microsoft/winget-pkgs/pull/411316) | Submit a fresh version; wait for Microsoft's validation and merge |
| Nix | [Source definition](../flake.nix) | CI builds the source package; version now follows Cargo.toml, with explicit Linux D-Bus and test dependencies |
| Website | [Pages workflow](../.github/workflows/pages.yml) | Deploy updated docs when main is authorized for publication |

The repository has the expected registry secret names and enabled AUR,
Chocolatey and Snap switches. No credential values were inspected. Required
reviewers are configured for both `release` and `registry` environments.

## Publication boundary

Candidate version/tag: **0.2.0 / v0.2.0**. Publishing updates main and the stable
tag, deploys website changes, hosts six native targets, and runs the existing
registry/submission jobs behind their environment protections. Git pushes and
store publication require explicit authorization under the supplied AGENTS.md
instructions; preparing the candidate does not bypass that boundary.

## Validation

- Rust formatting, Clippy and 144 automated tests pass; the live-account test
  remains opt-in.
- ShellCheck, Actionlint, metadata synchronization and deterministic cargo-dist
  workflow regeneration pass.
- The release plan contains all six targets with third-party notices in each.
- Manifest rendering rejects invalid versions and duplicate checksums. Asset
  verification rejects corrupted bytes.
- Disposable package fixtures build both Debian and RPM architectures. Debian
  metadata and archive contents confirm version, architecture, D-Bus dependency
  and third-party notices.
- Native macOS ARM64 archive and Cargo source-package checks are the final local
  candidate checks. Cross-platform execution runs after the authorized tag push.
- Nix is unavailable on this workstation. CI now builds its default package,
  with explicit D-Bus, Git/Python dependencies and sandbox-compatible fixture
  interpreters; local Nix execution has not been verified here.

External channel acceptance remains visible as a separate release follow-up.
The exact local commit and candidate artifacts are reported after validation.
