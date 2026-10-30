import { createHash } from 'node:crypto';
import { appendFileSync, readFileSync } from 'node:fs';
import { basename, resolve } from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';

const [archiveArg, tag, checksumFile] = process.argv.slice(2);
const mode = process.env.NPM_PUBLISH_MODE || 'stage';
if (!archiveArg || !checksumFile || !/^v\d+\.\d+\.\d+$/.test(tag || '') || !['stage', 'direct'].includes(mode)) {
  throw new Error('Usage: publish-npm.mjs ARCHIVE vMAJOR.MINOR.PATCH CHECKSUMS; NPM_PUBLISH_MODE=stage|direct');
}
const archive = resolve(archiveArg);
const version = tag.slice(1);
const bytes = readFileSync(archive);
const entries = readFileSync(checksumFile, 'utf8').split(/\r?\n/).filter(Boolean)
  .map(line => line.match(/^([a-fA-F0-9]{64})\s+\*?(.+)$/))
  .filter(entry => entry && entry[2].replace(/^\.\//, '') === basename(archive));
if (entries.length !== 1 || createHash('sha256').update(bytes).digest('hex') !== entries[0][1].toLowerCase()) {
  throw new Error('Missing, duplicate, or mismatched npm release checksum');
}
const manifest = JSON.parse(execFileSync('tar', ['-xOzf', archive, 'package/package.json'], { encoding: 'utf8' }));
if (manifest.name !== 'gitside' || manifest.version !== version || manifest.private) {
  throw new Error('npm package name, version, or visibility does not match the requested release');
}
const shasum = createHash('sha1').update(bytes).digest('hex');

function npm(args) {
  const result = spawnSync('npm', args, { encoding: 'utf8', timeout: 120_000, maxBuffer: 4 * 1024 * 1024 });
  if (result.stderr) process.stderr.write(result.stderr);
  if (result.error || result.status !== 0) throw new Error(`npm ${args.slice(0, 2).join(' ')} failed`, { cause: result.error });
  return result.stdout;
}

async function published() {
  const response = await fetch(`https://registry.npmjs.org/gitside/${version}`, { signal: AbortSignal.timeout(30_000) });
  if (response.status === 404) return false;
  if (!response.ok) throw new Error(`npm registry lookup failed: HTTP ${response.status}`);
  const data = await response.json();
  if (data.name !== 'gitside' || data.version !== version || data.dist?.shasum !== shasum) {
    throw new Error('The published version differs from the verified release archive');
  }
  return true;
}

function pending() {
  const stages = JSON.parse(npm(['stage', 'list', 'gitside', '--json']));
  if (!Array.isArray(stages)) throw new Error('Unexpected npm staged-package response');
  const matches = stages.filter(stage => stage.packageName === 'gitside' && stage.version === version);
  if (matches.length > 1) throw new Error('Multiple staging records exist for this version');
  const stage = matches[0];
  if (stage && (stage.shasum !== shasum || stage.tag !== 'latest' || !/^[a-f0-9-]{36}$/i.test(stage.id || ''))) {
    throw new Error('An existing stage differs from this archive or its latest tag; maintainer review is required');
  }
  return stage;
}

function report(status, stage) {
  const result = { package: 'gitside', version, status, shasum };
  if (stage) {
    result.stageId = stage.id;
    result.approveCommand = `npm stage approve ${stage.id}`;
  }
  console.log(JSON.stringify(result, null, 2));
  if (process.env.GITHUB_OUTPUT) {
    appendFileSync(process.env.GITHUB_OUTPUT, `status=${status}\nstage_id=${stage?.id || ''}\n`);
  }
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `## npm gitside ${version}\n\nStatus: **${status}**\n\n` +
      (stage ? `Awaiting maintainer 2FA approval in npm → Staged Packages.\n\nStage: \`${stage.id}\`\n\nCLI alternative: \`${result.approveCommand}\`\n` : 'The verified release archive is public on npm.\n'));
  }
}

if (await published()) {
  report('published');
} else {
  if (mode === 'stage') {
    let stage = pending();
    if (!stage) {
      npm(['stage', 'publish', archive, '--tag', 'latest', '--access', 'public', '--provenance', '--ignore-scripts', '--json']);
      stage = pending();
    }
    if (!stage) throw new Error('npm accepted the upload but no matching staging record is visible');
    report('staged', stage);
  } else {
    npm(['publish', archive, '--tag', 'latest', '--access', 'public', '--provenance', '--ignore-scripts']);
    let visible = false;
    for (let attempt = 0; attempt < 15; attempt++) {
      if (await published()) { visible = true; break; }
      await new Promise(resolveDelay => setTimeout(resolveDelay, 2000));
    }
    if (!visible) throw new Error('npm accepted publication but its public version is not yet visible');
    report('published');
  }
}
