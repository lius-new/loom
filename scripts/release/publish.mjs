import fs from 'node:fs';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {parseArgs} from 'node:util';
import {ROOT, commitId, generateReleaseNotes, validateVersion, writeReleaseNotes} from './notes.mjs';

function gh(args) {
  const result = spawnSync('gh', args, {encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe']});
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

export function uploadToDraft({version, repo, assets}, run = gh) {
  const tag = `v${version}`;
  const existing = run(['release', 'view', tag, '--repo', repo, '--json', 'isDraft', '--jq', '.isDraft']);
  if (existing.status !== 0) throw new Error(`Draft ${tag} is unavailable; check the Prepare release draft workflow. ${existing.stderr || existing.stdout}`);
  if (existing.stdout.trim() !== 'true') throw new Error(`${tag} is already published; refusing to change its notes or assets.`);
  const uploaded = run(['release', 'upload', tag, ...assets, '--clobber', '--repo', repo]);
  if (uploaded.status !== 0) throw new Error(`Could not upload release assets: ${uploaded.stderr || uploaded.stdout}`);
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
    else uploadToDraft(prepareUpload(values));
    console.log(`Prepared draft v${values.version}. Review all platform packages and notes before publishing it on GitHub.`);
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
