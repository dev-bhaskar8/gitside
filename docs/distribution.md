# Packages and releases

GitHub Releases hosts Gitside's native artifacts and checksums. The same tagged
source feeds Cargo, npm, Homebrew, Scoop, AUR, Windows package manifests, native
Linux packages, Snap, and the Nix source package. Store availability is tracked
separately from successful builds.

## Installation matrix

Public channel state was checked on October 31, 2026, after publishing 0.2.0.
Registry acceptance and store listing remain separate from the GitHub release.

| Channel | Install | Verified public state |
| --- | --- | --- |
| Homebrew | `brew install dev-bhaskar8/tap/gitside` | 0.2.0; macOS and Linux, x64/ARM64 |
| Cargo | `cargo install gitside --locked` | 0.1.2; 0.2.0 upload rejected with HTTP 403 authentication failure |
| npm | `npm install --global gitside` | 0.1.2; 0.2.0 publication returned a permission/not-found error |
| Scoop | `scoop bucket add gitside https://github.com/dev-bhaskar8/scoop-bucket`, then `scoop install gitside` | 0.2.0; Windows x64/ARM64 |
| AUR | `paru -S gitside-bin` | 0.2.0-1; x64/ARM64 |
| Nix | `nix run github:dev-bhaskar8/gitside` | 0.2.0 source package; CI builds the default package |
| Direct | [GitHub Releases](https://github.com/dev-bhaskar8/gitside/releases/tag/v0.2.0) | 0.2.0; six archives, shell/PowerShell installers, .deb/.rpm and .nupkg |

The npm package is a native-binary installer. AUR, Nix, and the Homebrew tap are
independent channels, not inclusion in Arch, Nixpkgs, or Homebrew's core repositories.

### Configured channels awaiting publication or review

- **Chocolatey:** the 0.2.0 upload succeeded and its `.nupkg` is attached to the
  GitHub release. The anonymous Chocolatey feed still returned no public entry.
- **Snap:** no public store release is available. The initial 0.2.0 builds failed
  because the pinned Rust 1.85 compiler cannot compile a dependency's newer
  syntax. Both architectures then built successfully with stable Rust, but the
  store rejected obsolete Candid credentials. A [final packaging retry](https://github.com/dev-bhaskar8/gitside/actions/runs/41134726622)
  adds automatic ELF patching, installed-package execution checks and retention
  of the built snaps before store publication. Refresh SNAPCRAFT_STORE_CREDENTIALS
  using modern `snapcraft export-login` with SNAPCRAFT_STORE_AUTH unset. Store
  review and classic-confinement approval remain separate requirements.
- **WinGet:** [0.2.0 submission](https://github.com/microsoft/winget-pkgs/pull/466584)
  is open. Microsoft's bot requires the maintainer's CLA signature before merge;
  package validation and upstream acceptance are also required.

See the [0.2.0 release record](release-0.2.0.md) for evidence and next actions.
PPA, COPR, and other official OS repositories are not currently configured.

Use current stable Rust for source builds. A dependency uses syntax unavailable
in Rust 1.85, despite the older lower bound in the original 0.2.0 source metadata.

## Release architecture

The six native targets are Apple Silicon/Intel macOS, GNU Linux x64/ARM64, and
Windows MSVC x64/ARM64. Other Rust-supported hosts can build from source.

1. A stable `vMAJOR.MINOR.PATCH` tag starts cargo-dist 0.32.0.
2. Local and global build jobs produce native archives, installers, SHA-256
   checksums, a CycloneDX SBOM, and GitHub provenance attestations.
3. The protected `release` environment gates hosting on GitHub Releases.
4. A directly called reusable workflow downloads and executes all six hosted
   binaries, checks their versions/checksums, and verifies archive notices.
5. After verification, the protected `registry` environment gates Cargo, npm,
   Homebrew, Scoop, AUR, Chocolatey, Snap and WinGet publication/submission jobs.
6. The package workflow verifies its downloaded native archives, builds .deb/.rpm
   packages, renders registry manifests, and attaches packages and attestations.
   A Nix source build is checked in CI; the website deploys from main separately.

The direct workflow calls matter: a release created using `GITHUB_TOKEN` does
not trigger another `release: published` workflow. Package and smoke workflows
also retain manual/release entry points for targeted retries and external releases.
Release and registry reviewer protections remain enabled.

macOS builders are pinned to supported macOS 15 images. If infrastructure breaks
an existing tag before it is hosted, dispatch the current release workflow with
`-f tag=vMAJOR.MINOR.PATCH`. The release hosting environment accepts only `v*` tag
refs, so use a separate orchestration tag such as `v-release-recovery-20261031`
on the reviewed workflow commit, rather than dispatching from main. This tag
does not match the version-tag push trigger. The workflow checks out the original
version tag for every source build and Cargo publication; the release tag never
moves, and the environment policies and reviewer protections remain unchanged.

Linux packages declare their D-Bus runtime dependency for secure key storage;
Nix also declares the Git/Python check tools, and CI builds the default Nix package
rather than only evaluating the flake.

Native archives and package installers include `third-party-notices.md` for the
adapted OpenCode and T3 Code code. Cargo includes it in the source package; AUR,
Debian/RPM, Nix, and Snap install it in their license directories. Google's
Antigravity runtime is downloaded only when requested by an end user and is
not bundled in any Gitside package.

## Credentials and channel switches

Registry credentials live in the protected `registry` environment, never in source.

| Publisher | Environment secret | Repository variable |
| --- | --- | --- |
| Cargo | `CARGO_REGISTRY_TOKEN` | — |
| npm | `NPM_TOKEN` | — |
| Homebrew | `HOMEBREW_TAP_TOKEN` | — |
| Scoop | `SCOOP_BUCKET_TOKEN` | — |
| WinGet | `WINGET_TOKEN` | — |
| AUR | `AUR_SSH_PRIVATE_KEY` | `ENABLE_AUR=true` |
| Chocolatey | `CHOCOLATEY_API_KEY` | `ENABLE_CHOCOLATEY=true` |
| Snap | `SNAPCRAFT_STORE_CREDENTIALS` | `ENABLE_SNAP=true` |

The release confirmed these secret names and switch values without reading secret
values. Cargo and npm rejected their saved publishing credentials. The Homebrew,
Scoop, and WinGet GitHub tokens were also rejected; those channels were recovered
using the maintainer's authenticated local GitHub session. Snap's obsolete Candid
credentials were also rejected. Refresh all six affected credentials before
relying on the automatic publishers again. AUR and Chocolatey
publication succeeded with their configured credentials.

Snap's recipe uses Canonical's [automatic ELF patching](https://ubuntu.com/docs/snapcraft/9/how-to/debugging/use-the-classic-linter/#enable-automatic-elf-file-patching)
for the core24 loader and library paths. Its publisher uses the current
[exported-credential authentication](https://ubuntu.com/docs/snapcraft/9/how-to/publishing/authenticate/#export-snapcraft-account-credentials)
flow, without the retired Candid override.

## Maintainer release checklist

1. Update Cargo.toml, Cargo.lock, the Snap version and CHANGELOG.md. The Nix
   version is derived from Cargo.toml. Keep benchmark measurements dated rather
   than relabelling older measurements as the new release.
2. Update README.md, website documentation, provider setup and release notes.
3. Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
   `cargo test --locked`, and `cargo package --locked`.
4. Run `scripts/check-release-metadata.sh`, `scripts/test-packaging.sh`,
   `shellcheck scripts/*.sh`, and
   `SHELLCHECK_OPTS='--exclude=SC2001,SC2086,SC2129' actionlint`.
5. Run `scripts/check-release-workflow.sh` and inspect
   `dist plan --tag vMAJOR.MINOR.PATCH --output-format=json`.
6. Commit locally, review the candidate, and obtain explicit remote-push
   authorization before publishing the branch/tag when working under AGENTS.md.
7. Publish the stable tag. Review and approve the protected `release` deployment;
   inspect the hosted artifacts and six native verification jobs, then approve
   `registry` publication. Do not disable either protection to unblock a release.
8. Verify each public channel before announcing that it is available. A successful
   store submission can still require moderation or a separate upstream merge.

## Targeted retries

The manual package workflow reuses the selected GitHub release's canonical native
artifacts. `source_ref` selects corrected packaging metadata; omitting it uses the
tagged source. For example, after the candidate has been published:

```sh
gh workflow run packages.yml --ref main \
  -f tag=v0.2.0 -f source_ref=main -f target=aur
```

Other targets are `scoop`, `chocolatey`, `winget`, `snap`, and `all`. Resolve external
store blockers before retrying them. Scoop and AUR updates skip an unchanged
manifest; WinGet submissions retain prior versions. Do not treat a submission
attempt as a successful public release.
