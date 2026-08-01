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
}

replacements.each do |from, to|
  count = source.scan(from).length
  abort "expected one release workflow match for #{from.inspect}, found #{count}" unless count == 1
  source = source.sub(from, to)
end

File.write(path, source)
RUBY
