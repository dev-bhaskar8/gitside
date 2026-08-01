# Packages and releases

GitHub Releases is the source of truth for every Gitside package. A stable tag
builds the six native targets, records SHA-256 checksums, generates a CycloneDX
SBOM, and attaches GitHub OIDC provenance before any registry publication runs.

## Installation matrix

The commands below become usable after `v0.1.0` is published and the named
registry has accepted the package.

| Channel | Command | Platforms |
| --- | --- | --- |
| Homebrew | `brew install dev-bhaskar8/tap/gitside` | macOS, Linux |
| Cargo | `cargo install gitside --locked` | Rust-supported systems |
| WinGet | `winget install DevBhaskar8.Gitside` | Windows x64, ARM64 |
| Scoop | `scoop bucket add gitside https://github.com/dev-bhaskar8/scoop-bucket && scoop install gitside` | Windows x64, ARM64 |
| Chocolatey | `choco install gitside` | Windows x64, ARM64 |
| npm | `npm install --global gitside` | macOS, Linux, Windows |
| AUR | `paru -S gitside-bin` | Arch Linux x64, ARM64 |
| Nix | `nix run github:dev-bhaskar8/gitside` | macOS, Linux |
| Snap | `sudo snap install gitside --classic` | Linux x64, ARM64 |
| Direct | Download an installer or archive from [GitHub Releases](https://github.com/dev-bhaskar8/gitside/releases) | macOS, Linux, Windows |

The npm package is a thin native-binary installer, not a JavaScript rewrite.
Debian and RPM packages are attached directly to each release. PPA, COPR, and
official operating-system repositories are submitted only after the first
stable package has been exercised in the independent channels above.

## Supported release targets

- Apple silicon and Intel macOS
- GNU Linux on x86-64 and ARM64
- Windows MSVC on x86-64 and ARM64

Other platforms can install from source with `cargo install gitside --locked`.

## Release architecture

1. `cargo-dist` builds immutable native artifacts from a `vMAJOR.MINOR.PATCH` tag.
2. The protected `release` environment requires approval before hosting them.
3. GitHub records checksums, dependency SBOMs, and build provenance.
4. The protected `registry` environment publishes crates.io, npm, Homebrew,
   Scoop, Chocolatey, AUR, WinGet, and Snap metadata from those same artifacts.
5. `.deb` and `.rpm` packages and rendered registry manifests are attached to
   the release so every downstream package can be audited or reproduced.

The source repository never stores registry credentials. GitHub environment
secrets provide the minimum token required by each publisher.

## Maintainer release checklist

Before the first release, reserve `gitside` on crates.io, npm, Chocolatey, AUR,
and Snapcraft; create the Homebrew tap and Scoop bucket; fork WinGet; and add
these secrets to the `registry` GitHub environment:

- `CARGO_REGISTRY_TOKEN`
- `NPM_TOKEN`
- `HOMEBREW_TAP_TOKEN`
- `SCOOP_BUCKET_TOKEN`
- `CHOCOLATEY_API_KEY`
- `AUR_SSH_PRIVATE_KEY`
- `WINGET_TOKEN`
- `SNAPCRAFT_STORE_CREDENTIALS`

Then:

1. Update `CHANGELOG.md` and keep the Cargo and Snap versions identical.
2. Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
   `cargo test --locked`, `cargo package --locked`, and
   `scripts/test-packaging.sh`.
3. Run `scripts/check-release-workflow.sh` to reproduce and compare the
   protected release workflow.
4. Push `vMAJOR.MINOR.PATCH`. Review the release plan and approve `release`.
5. Inspect the hosted checksums and attestations, then approve `registry`.
6. Verify each install command in the table before announcing the release.

Publishing is intentionally not automatic from `main`: a stable tag and two
explicit environment approvals are required.
