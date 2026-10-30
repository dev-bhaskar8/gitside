#!/bin/sh
set -eu

workflow=${1:-.github/workflows/release.yml}

ruby - "$workflow" <<'RUBY'
path = ARGV.fetch(0)
source = File.read(path)

replacements = {
  "on:\n  pull_request:" => "on:\n  workflow_dispatch:\n    inputs:\n      tag:\n        description: Existing stable release tag to build without moving it\n        required: true\n        type: string\n  pull_request:",
  "github.ref_name" => "(inputs.tag || github.ref_name)",
  "  host:\n    needs:" => "  host:\n    environment: release\n    needs:",
  "  publish-homebrew-formula:\n    needs:" => "  publish-homebrew-formula:\n    environment: registry\n    needs:",
  "  publish-npm:\n    needs:" => "  publish-npm:\n    environment: registry\n    permissions:\n      contents: read\n      id-token: write\n    needs:",
  '      GITHUB_USER: "axo bot"' => '      GITHUB_USER: "dev-bhaskar8"',
  '      GITHUB_EMAIL: "admin+bot@axo.dev"' => '      GITHUB_EMAIL: "vaas.ygg@gmail.com"',
  "          node-version: '20.x'" => "          node-version: '24.x'",
  '            npm publish --access public "./npm/${pkg}"' => '            npm publish --access public --provenance "./npm/${pkg}"',
  '${{ steps.cargo-cyclonedx.output.paths }}' => '${{ steps.cargo-cyclonedx.outputs.paths }}',
  '            echo "$HOME/.cargo/bin" >> $GITHUB_PATH' => '            echo "$HOME/.cargo/bin" >> "$GITHUB_PATH"',
  "          echo \"paths<<EOF\" >> \"$GITHUB_OUTPUT\"\n          dist print-upload-files-from-manifest --manifest dist-manifest.json >> \"$GITHUB_OUTPUT\"\n          echo \"EOF\" >> \"$GITHUB_OUTPUT\"" => "          {\n            echo \"paths<<EOF\"\n            dist print-upload-files-from-manifest --manifest dist-manifest.json\n            echo \"EOF\"\n          } >> \"$GITHUB_OUTPUT\"",
  "          echo \"paths<<EOF\" >> \"$GITHUB_OUTPUT\"\n          jq --raw-output \".upload_files[]\" dist-manifest.json >> \"$GITHUB_OUTPUT\"\n          echo \"EOF\" >> \"$GITHUB_OUTPUT\"" => "          {\n            echo \"paths<<EOF\"\n            jq --raw-output \".upload_files[]\" dist-manifest.json\n            echo \"EOF\"\n          } >> \"$GITHUB_OUTPUT\"",
  "          echo \"$ANNOUNCEMENT_BODY\" > $RUNNER_TEMP/notes.txt\n\n          gh release create \"${{ needs.plan.outputs.tag }}\" --target \"$RELEASE_COMMIT\" $PRERELEASE_FLAG --title \"$ANNOUNCEMENT_TITLE\" --notes-file \"$RUNNER_TEMP/notes.txt\" artifacts/*" => "          echo \"$ANNOUNCEMENT_BODY\" > \"$RUNNER_TEMP/notes.txt\"\n\n          prerelease_args=()\n          if [[ -n \"$PRERELEASE_FLAG\" ]]; then\n            prerelease_args+=(\"$PRERELEASE_FLAG\")\n          fi\n          gh release create \"${{ needs.plan.outputs.tag }}\" --target \"$RELEASE_COMMIT\" \"${prerelease_args[@]}\" --title \"$ANNOUNCEMENT_TITLE\" --notes-file \"$RUNNER_TEMP/notes.txt\" artifacts/*",
  '            name=$(echo "$filename" | sed "s/\.rb$//")' => '            name=${filename%.rb}',
  '            git commit -m "${name} ${version}"' => '            git diff --cached --quiet || git commit -m "${name} ${version}"',
  "      - name: Install dependencies\n        run: |\n          \${{ matrix.packages_install }}" => "      - name: Install dependencies\n        run: |\n          \${{ matrix.packages_install }}\n      - name: Install Linux credential-store dependencies\n        if: \${{ contains(join(matrix.targets, ','), 'unknown-linux-gnu') }}\n        run: |\n          if [ \"\$(id -u)\" -eq 0 ]; then\n            apt-get update\n            apt-get install -y pkg-config libdbus-1-dev\n          else\n            sudo apt-get update\n            sudo apt-get install -y pkg-config libdbus-1-dev\n          fi",
}

replacements.each do |from, to|
  count = source.scan(from).length
  expected = from == "github.ref_name" ? 3 : 1
  abort "expected #{expected} release workflow matches for #{from.inspect}, found #{count}" unless count == expected
  source = source.gsub(from, to)
end

# A manual recovery uses current orchestration with the immutable tagged source.
checkout = "          persist-credentials: false\n          submodules: recursive"
abort "expected five release source checkouts" unless source.scan(checkout).length == 5
source = source.gsub(checkout, "          ref: ${{ inputs.tag || github.ref }}\n" + checkout)

plan_env = "    env:\n      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}"
abort "missing plan environment" unless source.include?(plan_env)
source = source.sub(plan_env, "    env:\n      RELEASE_TAG: ${{ inputs.tag || github.ref_name }}\n      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}")
plan_command = /          dist \$\{\{ .*? \}\} --output-format=json > plan-dist-manifest.json/
abort "missing release planning command" unless source.scan(plan_command).length == 1
source = source.sub(plan_command, <<~BASH.lines.map { |line| "          #{line}" }.join.chomp)
  plan_args=(plan)
  if [[ "$GITHUB_EVENT_NAME" != pull_request ]]; then
    if [[ ! "$RELEASE_TAG" =~ ^v[0-9]+\\.[0-9]+\\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]]; then
      echo "Expected a version tag, got: $RELEASE_TAG" >&2
      exit 1
    fi
    git rev-parse --verify "refs/tags/$RELEASE_TAG^{commit}"
    plan_args=(host --steps=create "--tag=$RELEASE_TAG")
  fi
  dist "${plan_args[@]}" --output-format=json > plan-dist-manifest.json
BASH

# Old tags may still plan macOS 14. Use a supported equivalent during recovery.
runner = '    runs-on: ${{ matrix.runner }}'
abort "missing native artifact runner" unless source.scan(runner).length == 1
source = source.sub(runner, "    runs-on: ${{ matrix.runner == 'macos-14' && 'macos-15' || matrix.runner }}")

# A GITHUB_TOKEN-created release cannot trigger release:published workflows.
# Call verification and package publication directly, keeping both protections.
verification = <<~YAML
  verify-release:
    needs:
      - plan
      - host
    uses: ./.github/workflows/platform-smoke.yml
    with:
      tag: ${{ needs.plan.outputs.tag }}
    permissions:
      contents: read

YAML
needle = "  publish-homebrew-formula:\n"
abort "missing Homebrew publication job" unless source.scan(needle).length == 1
source = source.sub(needle, verification.lines.map { |line| line.strip.empty? ? line : "  #{line}" }.join + needle)

%w[publish-homebrew-formula publish-npm custom-publish-crate custom-packages].each do |job|
  pattern = /(  #{Regexp.escape(job)}:\n.*?    needs:\n      - plan\n      - host)(?=\n)/m
  abort "missing release verification dependency for #{job}" unless source.scan(pattern).length == 1
  source = source.sub(pattern) { "#{$1}\n      - verify-release" }
end

custom_permissions = "    permissions:\n      \"id-token\": \"write\"\n      \"packages\": \"write\""
abort "expected two custom publisher permission blocks" unless source.scan(custom_permissions).length == 2
source = source.gsub(custom_permissions, "    permissions:\n      \"contents\": \"read\"\n      \"id-token\": \"write\"\n      \"packages\": \"write\"")

# The called package workflow uploads release assets and creates attestations.
package_start = source.index("  custom-packages:\n") or abort "missing package publication job"
package_end = source.index("\n  announce:", package_start) or abort "missing announce job"
package_job = source[package_start...package_end]
package_job = package_job.sub('"contents": "read"', '"contents": "write"')
package_job = package_job.sub('"id-token": "write"', '"id-token": "write"' + "\n      attestations: write")
source[package_start...package_end] = package_job

File.write(path, source)
RUBY
