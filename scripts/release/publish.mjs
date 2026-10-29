import fs from 'node:fs';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {parseArgs} from 'node:util';
import {setTimeout as delay} from 'node:timers/promises';
import {ROOT, commitId, generateReleaseNotes, validateVersion, writeReleaseNotes} from './notes.mjs';

function gh(args, {timeout = args[1] === 'view' ? 30000 : 300000} = {}) {
  const result = spawnSync('gh', args, {encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout});
  if (result.error) throw result.error;
  return result;
}

export function createDraft({version, repo, notes}, run = gh) {
  const tag = `v${version}`;
  const view = () => run(['release', 'view', tag, '--repo', repo, '--json', 'isDraft', '--jq', '.isDraft']);
  let existing = view();
  if (existing.status !== 0) {
    const created = run(['release', 'create', tag, '--repo', repo, '--draft', '--verify-tag', '--title', `Loom ${version}`, '--notes-file', notes]);
    if (created.status !== 0) {
      // Recover a competing manual creation without replacing its edited notes.
      existing = view();
      if (existing.status !== 0) throw new Error(`Could not create release draft: ${created.stderr || created.stdout}`);
    } else existing = view();
  }
  if (existing.status !== 0) throw new Error(`Could not verify release draft: ${existing.stderr || existing.stdout}`);
  if (existing.stdout.trim() !== 'true') throw new Error(`${tag} is already published; refusing to change its notes or assets.`);
}

// Packaging and draft preparation are independent workflows. Wait only for a
// missing draft or a transient API failure; authentication/configuration errors
// and already published releases must still fail without uploading anything.
export async function uploadToDraft({version, repo, assets}, run = gh,
  {timeoutMs = 300000, pollMs = 2000, wait = delay, now = Date.now} = {}) {
  if (timeoutMs < 0 || !(pollMs > 0)) throw Error('Invalid draft wait budget');
  const tag = `v${version}`, deadline = now() + timeoutMs;
  for (;;) {
    let existing;
    try {
      existing = await run(['release', 'view', tag, '--repo', repo, '--json', 'isDraft', '--jq', '.isDraft'],
        {timeout:Math.max(1, Math.min(30000, deadline - now()))});
    } catch (error) {
      if (!['ETIMEDOUT','ECONNRESET','EAI_AGAIN'].includes(error.code)) throw error;
      existing = {status:1, stderr:'temporary failure: ' + error.message};
    }
    if (existing.status === 0) {
      if (existing.stdout.trim() !== 'true') throw Error(`${tag} is already published; refusing to change its notes or assets.`);
      break;
    }
    const reason = existing.stderr || existing.stdout || 'unknown GitHub error';
    const transient = /release not found|^not found$|HTTP (?:404|429|5\d\d)|connection reset|timed? out|TLS handshake timeout|temporary failure/i.test(reason);
    const remaining = deadline - now();
    if (/HTTP (?:401|403)|permission denied|bad credentials|authentication/i.test(reason) || !transient || remaining <= 0) throw Error(`Draft ${tag} is unavailable; check the Prepare release draft workflow. ${reason}`);
    await wait(Math.min(pollMs, remaining));
  }
  const uploaded = await run(['release', 'upload', tag, ...assets, '--clobber', '--repo', repo]);
  if (uploaded.status !== 0) throw Error(`Could not upload release assets: ${uploaded.stderr || uploaded.stdout}`);
}

function verifySource({root = ROOT, version, target = 'HEAD', repo}) {
  validateVersion(version);
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo ?? '')) throw new Error(`Invalid GitHub repository: ${repo}`);
  const manifest = fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8');
  const packageVersion = manifest.split(/^\[/m).find(section => section.startsWith('package]'))?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (packageVersion !== version) throw new Error(`Requested ${version}, but Cargo.toml is ${packageVersion}`);
  const targetId = commitId(root, target);
  let tagId;
  try { tagId = commitId(root, `refs/tags/v${version}`); }
  catch { throw new Error(`Tag v${version} must already exist. This command does not create tags.`); }
  if (tagId !== targetId) throw new Error(`Tag v${version} does not point to the build commit.`);
  return targetId;
}

export function prepareDraft({root = ROOT, version, target = 'HEAD', repo, notes}) {
  const targetId = verifySource({root, version, target, repo});
  if (!notes) throw new Error('--notes is required when creating a draft.');
  const result = generateReleaseNotes({root, version, ref: targetId, repo});
  writeReleaseNotes(result, notes);
  return {version, repo, notes};
}

export function prepareUpload({root = ROOT, version, target = 'HEAD', repo, assets}) {
  verifySource({root, version, target, repo});
  const files = fs.readdirSync(assets, {withFileTypes: true}).filter(entry => entry.isFile()).map(entry => path.resolve(assets, entry.name)).sort();
  if (!files.length) throw new Error(`No release assets found in ${assets}`);
  return {version, repo, assets: files};
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const {values} = parseArgs({options: {create: {type: 'boolean', default: false}, version: {type: 'string'}, target: {type: 'string', default: 'HEAD'}, repo: {type: 'string'}, assets: {type: 'string', default: 'dist'}, notes: {type: 'string'}}});
    if (values.create) createDraft(prepareDraft(values));
    else await uploadToDraft(prepareUpload(values));
    console.log(`Prepared draft v${values.version}. Review all platform packages and notes before publishing it on GitHub.`);
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
