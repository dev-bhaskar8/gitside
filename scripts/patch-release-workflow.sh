#!/bin/sh
set -eu

workflow=${1:-.github/workflows/release.yml}

ruby - "$workflow" <<'RUBY'
path = ARGV.fetch(0)
source = File.read(path)

replacements = {
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
  "    permissions:\n      \"id-token\": \"write\"\n      \"packages\": \"write\"\n\n  announce:" => "    permissions:\n      \"contents\": \"read\"\n      \"id-token\": \"write\"\n      \"packages\": \"write\"\n\n  announce:",
}

replacements.each do |from, to|
  count = source.scan(from).length
  abort "expected one release workflow match for #{from.inspect}, found #{count}" unless count == 1
  source = source.sub(from, to)
end

File.write(path, source)
RUBY
