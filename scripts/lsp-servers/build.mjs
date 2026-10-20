// Builds the selected packs into archives, one per pack and platform, under
// .packaging/lsp-servers/dist. Nothing here publishes anything.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {WORK, RECIPES, PINS, PLATFORMS, assert, loadRecipes, loadPins, validatePins, validateLocks, inputDigest, packPlatforms,
  assemblePack, packRecord, assetName, createArchive, writeAtomic, options, selectPacks, main} from './lib.mjs';

export async function buildPacks(args = {}) {
  const recipes = loadRecipes(args.recipes || RECIPES), pins = loadPins(args.pins || PINS), work = args.work || WORK;
  validatePins(recipes, pins); validateLocks(recipes);
  const ids = selectPacks(recipes, args);
  const only = args.platforms ? args.platforms.split(',') : null;
  if (only) only.forEach(platform => assert([...PLATFORMS, 'any'].includes(platform), `Unknown platform: ${platform}`));
  fs.mkdirSync(work, {recursive: true});
  const lock = path.join(work, 'build.lock');
  const descriptor = fs.openSync(lock, 'wx'); fs.writeFileSync(descriptor, String(process.pid));
  try {
    const digest = inputDigest(recipes, pins), dist = path.join(work, 'dist'); fs.mkdirSync(dist, {recursive: true});
    const report = {schema: 1, complete: false, input: digest, selected: ids, packs: {}, failures: []};
    let partial = false;
    for (const id of ids) {
      const pack = recipes.packs[id], platforms = packPlatforms(pack);
      const wanted = platforms.filter(platform => !only || only.includes(platform) || (platform === 'any' && only.length));
      if (wanted.length !== platforms.length) partial = true;
      report.packs[id] = {version: pack.version, platforms: {}};
      for (const platform of wanted) {
        console.log(`Building ${id} ${platform}`);
        try {
          const directory = await assemblePack(recipes, pins, id, platform, {...args, work});
          const archive = path.join(dist, assetName(id, platform)), temporary = archive + '.partial';
          await createArchive(directory, temporary); fs.renameSync(temporary, archive);
          report.packs[id].platforms[platform] = packRecord(id, platform, archive);
        } catch (error) { report.failures.push({pack: id, platform, error: error.message}); console.error(`${id} ${platform}: ${error.message}`); }
      }
    }
    report.partial = partial;
    report.complete = report.failures.length === 0 && !partial;
    assert(inputDigest(recipes, pins) === digest, 'Recipe inputs changed during build; rerun the batch');
    writeAtomic(path.join(dist, 'build-report.json'), JSON.stringify(report, null, 2) + '\n');
    writeAtomic(path.join(dist, 'SHA256SUMS'), Object.values(report.packs).flatMap(pack => Object.values(pack.platforms)).map(r => `${r.sha256}  ${r.asset}`).sort().join('\n') + '\n');
    assert(report.failures.length === 0, `${report.failures.length} pack(s) failed; see ${path.join(dist, 'build-report.json')}`);
    return report;
  } finally { fs.closeSync(descriptor); fs.rmSync(lock, {force: true}); }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(async () => {
    const args = options(process.argv.slice(2));
    if (args.help) { console.log('node scripts/lsp-servers/build.mjs --all | --packs rust-analyzer,pyright [--platforms windows-x86_64] [--offline] [--work directory]'); return; }
    assert(args.all || args.packs, 'Select --all or --packs');
    const report = await buildPacks(args);
    if (report.partial) console.log('Partial build: only a complete build can be prepared for publication.');
  });
}
