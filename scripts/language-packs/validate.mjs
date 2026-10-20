import fs from 'node:fs';
import path from 'node:path';
import {prepareDependencies} from '../dependencies/ensure.mjs';
import {ROOT, WORK, assert, loadRecipes, selectPackages, run, options, main} from './lib.mjs';

main(async()=>{
  const args=options(process.argv.slice(2));
  if(args.help){console.log('node scripts/language-packs/validate.mjs [--all | --packages python,rust] [--offline]');return;}
  const recipes=loadRecipes(args.recipes),work=args.work||WORK;
  const report=JSON.parse(fs.readFileSync(path.join(work,'dist/build-report.json'),'utf8'));
  const ids=args.all||args.packages?selectPackages(recipes,args):report.selected;
  const prepared=await prepareDependencies({skia:false,offline:args.offline});
  run('cargo',['build','--locked',...(args.offline?['--offline']:[]),'--manifest-path',path.join(ROOT,'tests/language-runtime/Cargo.toml'),'--bin','language-pack-validator'],{env:prepared.env,stdio:'inherit'});
  const executable=path.join(ROOT,'tests/language-runtime/target/debug',process.platform==='win32'?'language-pack-validator.exe':'language-pack-validator');
  for(const id of ids){
    assert(report.packages[id],`No verified build artifact: ${id}`);
    const samples=path.join(work,id+'-validation.json');
    fs.writeFileSync(samples,JSON.stringify(Object.entries(recipes.packages[id].grammars).map(([grammar,g])=>({grammar,sample:path.join(ROOT,g.sample)}))));
    console.log(run(executable,[path.join(work,'stage'),samples]).trim());
  }
});
