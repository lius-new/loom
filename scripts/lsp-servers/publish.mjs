// Prepares, verifies and publishes the built packs, then promotes the catalog.
// Publication follows the language package rules: a clean checkout, a fixed
// resource tag that is never "latest", existing assets never overwritten, and the
// catalog under assets replaced only after the release is confirmed public.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {WORK, CATALOG, RECIPES, PINS, assert, HASH, RELEASE, readToml, loadRecipes, loadPins, inputDigest, fileHash,
  checkArtifacts, makeCatalog, validateCatalog, catalogToml, writeAtomic, run, options, main} from './lib.mjs';
import {GitHubReleases} from '../language-packs/publish.mjs';

const digestOfInputs = args => inputDigest(loadRecipes(args.recipes || RECIPES), loadPins(args.pins || PINS));
const assetsOf = (catalog, selected) => selected.flatMap(id => Object.values(catalog.packs[id].platforms).map(archive => archive.asset)).sort();

export function preparePublication(args) {
  const dist = path.join(args.work || WORK, 'dist'), reportFile = path.join(dist, 'build-report.json');
  const report = JSON.parse(fs.readFileSync(reportFile, 'utf8'));
  assert(report.schema === 1 && report.complete && !report.failures.length, 'Build every selected pack for every platform before preparing publication');
  assert(report.input === digestOfInputs(args), 'Build inputs changed; rebuild before publication');
  const catalogFile = args.catalog || CATALOG, recipes = loadRecipes(args.recipes || RECIPES);
  const current = validateCatalog(readToml(catalogFile));
  const repository = args.repository || current.distribution.repository;
  assert(current.distribution.repository === repository, 'Changing the release repository would invalidate existing pack URLs');
  const candidate = makeCatalog(recipes, report, {repository, release: args.release, current});
  checkArtifacts(candidate, dist, report.selected);
  for (const id of report.selected) for (const [platform, archive] of Object.entries(candidate.packs[id].platforms)) {
    const old = current.packs?.[id]?.platforms?.[platform];
    assert(!old || old.release !== archive.release || old.sha256 === archive.sha256, `Published ${id} ${platform} is different; choose a new resource tag`);
  }
  writeAtomic(path.join(dist, 'lsp-servers.toml'), catalogToml(candidate));
  const selectedAssets = assetsOf(candidate, report.selected);
  const sums = report.selected.flatMap(id => Object.values(candidate.packs[id].platforms)).map(a => `${a.sha256}  ${a.asset}`).sort().join('\n') + '\n';
  writeAtomic(path.join(dist, 'SHA256SUMS'), sums);
  const assets = [...selectedAssets, 'lsp-servers.toml', 'SHA256SUMS', 'build-report.json'];
  const plan = {schema: 1, repository, release: args.release, input: report.input, base: fileHash(catalogFile), packs: report.selected,
    assets: Object.fromEntries(assets.map(name => [name, fileHash(path.join(dist, name))]))};
  writeAtomic(path.join(dist, 'publication.json'), JSON.stringify(plan, null, 2) + '\n');
  return {candidate, plan};
}

export function verifyPublication(args) {
  const dist = path.join(args.work || WORK, 'dist'), plan = JSON.parse(fs.readFileSync(path.join(dist, 'publication.json'), 'utf8'));
  const catalog = validateCatalog(readToml(path.join(dist, 'lsp-servers.toml')));
  assert(plan.schema === 1 && RELEASE.test(plan.release), 'Invalid publication plan');
  assert(plan.repository === catalog.distribution.repository, 'Publication repository mismatch');
  if (args.release) assert(args.release === plan.release, 'Publication tag mismatch');
  if (args.repository) assert(args.repository === plan.repository, 'Publication repository mismatch');
  assert(plan.input === digestOfInputs(args), 'Build inputs changed; rebuild before publication');
  for (const id of plan.packs) for (const archive of Object.values(catalog.packs[id].platforms)) assert(archive.release === plan.release, `Candidate pack does not match publication: ${id}`);
  checkArtifacts(catalog, dist, plan.packs);
  const report = JSON.parse(fs.readFileSync(path.join(dist, 'build-report.json'), 'utf8'));
  assert(report.complete && report.input === plan.input && JSON.stringify(report.selected) === JSON.stringify(plan.packs), 'Publication build report mismatch');
  const expected = [...assetsOf(catalog, plan.packs), 'lsp-servers.toml', 'SHA256SUMS', 'build-report.json'].sort();
  assert(JSON.stringify(Object.keys(plan.assets).sort()) === JSON.stringify(expected), 'Unexpected publication assets');
  for (const [name, digest] of Object.entries(plan.assets)) assert(HASH.test(digest) && fileHash(path.join(dist, name)) === digest, `Publication file changed: ${name}`);
  return {dist, plan, catalog};
}

export class LspReleases extends GitHubReleases {
  create(tag, commit, notes) {
    run('gh', ['release', 'create', tag, '--repo', this.repository, '--target', commit, '--draft', '--latest=false', '--title', `Loom language servers — ${tag}`, '--notes-file', notes]);
    return this.draft(tag);
  }
}

export async function publishPrepared(args, github, {commit, requireClean = true} = {}) {
  const {dist, plan} = verifyPublication(args), catalogFile = args.catalog || CATALOG;
  assert(fileHash(catalogFile) === plan.base || fileHash(catalogFile) === plan.assets['lsp-servers.toml'], 'Installed publication manifest changed since preparation');
  if (requireClean) assert(run('git', ['status', '--porcelain']).length === 0, 'Commit the recipes, pins and lockfiles before publishing');
  commit ||= run('git', ['rev-parse', 'HEAD']).trim();
  assert(/^[a-f0-9]{40}$/.test(commit), 'Pin publication to a concrete repository commit');
  github ||= new LspReleases(plan.repository);
  const notes = path.join(dist, 'release-notes.md');
  writeAtomic(notes, `Language servers for Loom.\n\n${plan.packs.map(id => `- ${id}`).join('\n')}\n\nSource commit: ${commit}\n`);
  let release = github.lookup(plan.release);
  if (!release) release = github.create(plan.release, commit, notes);
  assert(release.tag_name === plan.release, 'GitHub returned a different resource tag');
  const downloads = path.join(dist, 'verify-' + plan.release);
  for (const [name, digest] of Object.entries(plan.assets)) {
    const existing = release.assets.find(asset => asset.name === name);
    if (existing) {
      const temporary = path.join(downloads, name.replace(/[^\w.-]/g, '_'));
      assert(github.assetDigest(plan.release, existing, temporary) === digest, `Remote asset differs: ${name}; existing assets are never overwritten`);
    } else {
      assert(release.draft, `Published release is missing ${name}; choose a new resource tag`);
      github.upload(plan.release, path.join(dist, name));
    }
  }
  release = github.lookup(plan.release);
  for (const [name, digest] of Object.entries(plan.assets)) {
    const asset = release.assets.find(item => item.name === name); assert(asset, `Upload missing on server: ${name}`);
    assert(github.assetDigest(plan.release, asset, path.join(downloads, 'final-' + name.replace(/[^\w.-]/g, '_'))) === digest, `Uploaded asset digest differs: ${name}`);
  }
  if (args['draft-only']) return {published: false, release: plan.release};
  if (release.draft) github.publish(plan.release);
  assert(github.lookup(plan.release)?.draft === false, 'Release publication was not confirmed');
  assert(fileHash(catalogFile) === plan.base || fileHash(catalogFile) === plan.assets['lsp-servers.toml'], 'Publication manifest changed during upload');
  writeAtomic(catalogFile, fs.readFileSync(path.join(dist, 'lsp-servers.toml')));
  return {published: true, release: plan.release, packs: plan.packs};
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main(async () => {
  const [command, ...argv] = process.argv.slice(2), args = options(argv);
  if (args.help || !command || command === '--help') { console.log('node scripts/lsp-servers/publish.mjs prepare --release lsp-servers-001\nnode scripts/lsp-servers/publish.mjs verify\nnode scripts/lsp-servers/publish.mjs publish [--draft-only]'); return; }
  if (command === 'prepare') console.log(JSON.stringify(preparePublication(args).plan, null, 2));
  else if (command === 'verify') console.log(`Verified ${verifyPublication(args).plan.release}; no remote changes`);
  else if (command === 'publish') console.log(JSON.stringify(await publishPrepared(args)));
  else throw Error(`Unknown publication command: ${command}`);
});
