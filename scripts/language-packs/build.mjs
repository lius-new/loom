import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {prepareDependencies} from '../dependencies/ensure.mjs';
import {ROOT, WORK, assert, loadRecipes, selectPackages, options, main, inputDigest,
  prepareTools, sourcePreparer, assemblePackage, packageRecord, createArchive, writeAtomic, run} from './lib.mjs';

export async function buildPackages(args = {}) {
  const recipes = loadRecipes(args.recipes), ids = selectPackages(recipes,args), work = args.work || WORK;
  fs.mkdirSync(work, {recursive:true});
  const lock = path.join(work, 'build.lock');
  const descriptor = fs.openSync(lock, 'wx'); fs.writeFileSync(descriptor, String(process.pid));
  try {
    const digest = inputDigest(recipes), tools = await prepareTools(recipes,{...args,work});
    const prepare = sourcePreparer(recipes,{...args,work});
    const dependencies = await prepareDependencies({skia:false, offline:args.offline});
    run('cargo', ['build','--locked',...(args.offline?['--offline']:[]),'--manifest-path',path.join(ROOT,'tests/language-runtime/Cargo.toml'),
      '--bin','language-pack-validator'], {env:dependencies.env, stdio:'inherit'});
    const validator = path.join(ROOT,'tests/language-runtime/target/debug',process.platform==='win32'?'language-pack-validator.exe':'language-pack-validator');
    const dist = path.join(work,'dist'); fs.mkdirSync(dist,{recursive:true});
    const report = {schema:1, complete:false, input:digest, selected:ids, packages:{}, failures:[]};
    for (const id of ids) {
      console.log(`Building ${id}`);
      try {
        const directory = await assemblePackage(recipes,id,prepare,tools,{...args,work});
        const samples = path.join(work,`${id}-samples.json`);
        fs.writeFileSync(samples,JSON.stringify(Object.entries(recipes.packages[id].grammars).map(([grammar,g])=>({grammar,sample:path.join(ROOT,g.sample)}))));
        const validation = run(validator,[path.dirname(directory),samples]);
        const archive = path.join(dist,`${id}.tar.gz`), temporary = archive + '.partial';
        await createArchive(directory,temporary); fs.renameSync(temporary,archive);
        report.packages[id] = {...packageRecord(id,recipes.packages[id],archive),validation:validation.trim().split(/\r?\n/).map(line=>JSON.parse(line))};
        console.log(`Verified ${id}`);
      } catch(error) {report.failures.push({package:id,error:error.message}); console.error(`${id}: ${error.message}`);}
    }
    report.complete = report.failures.length === 0;
    assert(inputDigest(recipes) === digest, 'Recipe inputs changed during build; rerun the batch');
    writeAtomic(path.join(dist,'build-report.json'),JSON.stringify(report,null,2)+'\n');
    writeAtomic(path.join(dist,'SHA256SUMS'),Object.values(report.packages).map(pkg=>`${pkg.sha256}  ${pkg.asset}`).sort().join('\n')+'\n');
    assert(report.complete, `${report.failures.length} package(s) failed; see ${path.join(dist,'build-report.json')}`);
    return report;
  } finally {fs.closeSync(descriptor); fs.rmSync(lock,{force:true});}
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(async()=>{const args=options(process.argv.slice(2));
    if(args.help){console.log('node scripts/language-packs/build.mjs --all | --packages rust,python [--offline] [--work directory]');return;}
    assert(args.all || args.packages, 'Select --all or --packages'); await buildPackages(args);
  });
}
