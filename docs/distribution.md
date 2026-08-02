# Packages and releases

GitHub Releases is the source of truth for every Gitside package. A stable tag
builds the six native targets, records SHA-256 checksums, generates a CycloneDX
SBOM, and attaches GitHub OIDC provenance before any registry publication runs.

## Installation matrix

The commands below are the currently available installation channels. Chocolatey
is published but its first version is still in moderation review; use the direct
Windows installer until it is accepted.

| Channel | Command | Platforms |
| --- | --- | --- |
| Homebrew | `brew install dev-bhaskar8/tap/gitside` | macOS, Linux |
| Cargo | `cargo install gitside --locked` | Rust-supported systems |
| Scoop | `scoop bucket add gitside https://github.com/dev-bhaskar8/scoop-bucket && scoop install gitside` | Windows x64, ARM64 |
| Chocolatey | `choco install gitside` | Windows x64, ARM64 (pending first-version review) |
| npm | `npm install --global gitside` | macOS, Linux, Windows |
| Nix | `nix run github:dev-bhaskar8/gitside` | macOS, Linux |
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
   Scoop, and Chocolatey metadata from those same artifacts.
5. `.deb` and `.rpm` packages and rendered registry manifests are attached to
   the release so every downstream package can be audited or reproduced.

The source repository never stores registry credentials. GitHub environment
secrets provide the minimum token required by each publisher.

## Maintainer release checklist

The core release can publish after reserving `gitside` on crates.io and npm,
creating the Homebrew tap and Scoop bucket, and adding these secrets to the
`registry` GitHub environment:

- `CARGO_REGISTRY_TOKEN`
- `NPM_TOKEN`
- `HOMEBREW_TAP_TOKEN`
- `SCOOP_BUCKET_TOKEN`

Chocolatey requires a protected registry credential. Add it and enable its
repository Actions variable:

| Publisher | Environment secret | Repository variable |
| --- | --- | --- |
| Chocolatey | `CHOCOLATEY_API_KEY` | `ENABLE_CHOCOLATEY=true` |

For example, after configuring Chocolatey:

```sh
gh variable set ENABLE_CHOCOLATEY --body true
gh workflow run packages.yml --ref main -f tag=v0.1.2 -f source_ref=main -f target=all
```

The manual run safely reuses the existing release. Scoop publication is
idempotent.

When a package's first version is still in Chocolatey moderation, publish that
same version again after fixing it instead of trying to push a newer version:

```sh
gh workflow run packages.yml --ref main -f tag=v0.1.0 -f source_ref=main -f target=chocolatey
```

Then:

1. Update `CHANGELOG.md` and keep all published package versions identical.
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
