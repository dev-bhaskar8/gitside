# Gitside 0.2.0 release record

Status checked on October 31, 2026. The [GitHub release](https://github.com/dev-bhaskar8/gitside/releases/tag/v0.2.0)
is published. Availability in each package registry is tracked separately below.

## Scope

- Eight named coding providers plus compatible ACP agents for Generate with AI.
- Managed, verified Antigravity runtime installation and personal Google sign-in
  inside setup, account model discovery, cancellation and expired-session recovery.
- Structured commit validation, existing CLI login reuse, bounded context/output,
  process cleanup and preservation of drafts edited during generation.
- Updated README, website, source/package metadata, license notices, and release
  workflow connections. The historical 0.1.2 benchmark remains explicitly dated.

## Distribution results

| Channel | Evidence | Result / remaining action |
| --- | --- | --- |
| GitHub/native archives | [0.2.0 release](https://github.com/dev-bhaskar8/gitside/releases/tag/v0.2.0) | Published six native targets, installers, checksums, source and SBOM; all six binaries passed hosted execution checks |
| Linux packages | [Release assets](https://github.com/dev-bhaskar8/gitside/releases/tag/v0.2.0) | Published two Debian and two RPM packages with provenance attestations |
| Cargo | [Registry entry](https://crates.io/crates/gitside), [publisher failure](https://github.com/dev-bhaskar8/gitside/actions/runs/41131024871/job/124327624358) | Still 0.1.2; 0.2.0 compiled and packaged, but upload returned HTTP 403 authentication failure; refresh CARGO_REGISTRY_TOKEN |
| npm | [Package](https://www.npmjs.com/package/gitside), [publisher failure](https://github.com/dev-bhaskar8/gitside/actions/runs/41131024871/job/124327624264) | Still 0.1.2; 0.2.0 publication returned E404 permission/not-found; refresh NPM_TOKEN or provide an authenticated local npm session |
| Homebrew | [Tap update](https://github.com/dev-bhaskar8/homebrew-tap/commit/60f1542) | Published 0.2.0 after recovering from rejected CI credentials |
| Scoop | [Bucket update](https://github.com/dev-bhaskar8/scoop-bucket/commit/728bab3) | Published 0.2.0 for both Windows architectures after recovering from rejected CI credentials |
| AUR | [Package](https://aur.archlinux.org/packages/gitside-bin), [publisher](https://github.com/dev-bhaskar8/gitside/actions/runs/41131024871/job/124328052361) | Published 0.2.0-1; the earlier maintenance blocker did not recur |
| Chocolatey | [Publisher](https://github.com/dev-bhaskar8/gitside/actions/runs/41131024871/job/124328052446) | Upload accepted and .nupkg attached to GitHub; anonymous public feed still returned no listed package |
| Snap | [Packaging retry](https://github.com/dev-bhaskar8/gitside/actions/runs/41134726622), [store rejection](https://github.com/dev-bhaskar8/gitside/actions/runs/41133657516) | Both architectures compiled with stable Rust; store rejected obsolete Candid credentials. Final retry adds ELF patching, runtime execution checks and retained release assets; refresh SNAPCRAFT_STORE_CREDENTIALS |
| WinGet | [0.2.0 submission](https://github.com/microsoft/winget-pkgs/pull/466584) | Submitted; Microsoft requires the maintainer's CLA signature before merge, plus package validation and upstream acceptance |
| Nix | [CI](https://github.com/dev-bhaskar8/gitside/actions/runs/41131020699), [source definition](../flake.nix) | Default 0.2.0 source package built successfully in CI |
| Website | [Pages deployment](https://github.com/dev-bhaskar8/gitside/actions/runs/41129782931), [live website](https://dev-bhaskar8.github.io/gitside/) | Deployed; homepage and setup documentation returned HTTP 200 and contain the new Google sign-in instructions |

Homebrew, Scoop and WinGet were recovered through the authenticated local GitHub
session. Their existing CI tokens still need refreshing before the next automatic
publication. No credential values were printed or copied into new secrets.

## Publication and recovery

Remote publication was explicitly authorized on October 31, 2026. Main and the
annotated `v0.2.0` tag were pushed. The release source remains commit
`e67269006c368813cfcd3eecf8001bfdbd0e8e51`; that tag was never moved.

The first release attempt encountered GitHub's macOS 14 retirement brownout.
Supported macOS 15 builders and an existing-tag recovery input were added in
`b60db2a`. The hosting environment permits only `v*` tag refs, so the reviewed
orchestration runs from `v-release-recovery-20261031` at `3bc4795`, while every
native build and Cargo publication checks out the original `v0.2.0` source.
The recovery tag does not match the automatic version-tag push trigger.

The [release run](https://github.com/dev-bhaskar8/gitside/actions/runs/41131024871)
passed every build and native verification job. Release and registry deployments
were reviewed and approved with the original reviewer and branch policies intact.
Its overall failure records rejected publisher credentials and the initial Snap
compiler mismatch; it does not indicate failed native binaries.

The Snap recipe correction is commit `379f563`. The targeted retry uses main's
corrected packaging metadata for version 0.2.0, with the Rust application source
unchanged. Commit `088fbe4` removes the retired Candid authentication override,
enables automatic ELF patching and retains verified Snap builds before store
upload. Both architectures compiled successfully in the earlier retry, but the
store rejected its obsolete credentials. Source builds should use current stable Rust: a dependency uses syntax
that Rust 1.85 cannot compile, despite the older lower bound in the tagged metadata.

## Validation

- Rust formatting, Clippy and 144 automated tests passed; the live-account test
  remains opt-in.
- ShellCheck, Actionlint, metadata synchronization, package fixtures and
  deterministic cargo-dist workflow regeneration passed. CI also built Nix.
- All six published archives passed checksum, executable-version and notice checks
  on their native platforms, before registry publication was approved.
- The generated npm installer was installed into a disposable local prefix and
  executed `gitside 0.2.0`; this verifies the installer artifact, not npm acceptance.
- GitHub provenance verification passed for the hosted macOS ARM64 archive.
- Homebrew's four native hashes and Scoop's two Windows hashes match the hosted
  release checksums. Both Windows ZIP executable paths match the WinGet manifests.
- Debian/RPM fixtures and production packages built for both architectures;
  fixtures validate version, architecture, D-Bus dependency and third-party notices.
- Cargo source packaging compiled successfully locally, in CI and in the publisher
  before the registry rejected its credential.

External acceptance and credential recovery remain separate release follow-ups.
