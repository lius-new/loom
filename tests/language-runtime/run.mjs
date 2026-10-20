import {prepareDependencies} from '../../scripts/dependencies/ensure.mjs';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
import {prepareWasmFixture} from './wasm-fixture.mjs';
import {legacyFixture} from './legacy-fixture.mjs';
const root=path.dirname(fileURLToPath(import.meta.url));
const args=process.argv.slice(2),i=args.indexOf('--packages');
if(i<0||!args[i+1])throw Error('Supply --packages with complete prebuilt artifacts; tests do not build grammar sources');
const packages=path.resolve(args[i+1]);
const {root:legacyPackages,collections}=legacyFixture(packages,path.join(root,'target/legacy-input'));
const option=name=>{const index=args.indexOf(name);return index<0?null:args[index+1];};
const extraPackage=option('--new-package'),sample=option('--sample');
if(Boolean(extraPackage)!==Boolean(sample))throw Error('Use --new-package and --sample together to verify a genuinely new external language');
const wasmPackages=await prepareWasmFixture(path.join(root,'target/wasm-input'),{module:option('--wasm-rust'),offline:args.includes('--offline')});
const env={...(await prepareDependencies({skia:false,offline:args.includes('--offline')})).env,LOOM_TEST_LANGUAGE_PACKS:packages,LOOM_TEST_LEGACY_PACKAGES:legacyPackages,LOOM_TEST_WASM_PACKAGES:wasmPackages};
const offline=args.includes('--offline')?['--offline']:[];
const cargo=command=>execFileSync('cargo',[...command,'--manifest-path',path.join(root,'Cargo.toml'),'--locked',...offline,'--target-dir',path.join(root,'target')],{cwd:root,env,stdio:'inherit'});
if(!args.includes('--probe-only'))cargo(['test','--no-fail-fast']);
cargo(['build','--bin','language-runtime-probe']);
const temp=fs.mkdtempSync(path.join(os.tmpdir(),'loom-installed-language-'));
try {
  const binary=path.join(temp,process.platform==='win32'?'probe.exe':'probe');
  fs.copyFileSync(path.join(root,'target/debug',process.platform==='win32'?'language-runtime-probe.exe':'language-runtime-probe'),binary);
  if(process.platform!=='win32')fs.chmodSync(binary,0o755);
  const digest=()=>createHash('sha256').update(fs.readFileSync(binary)).digest('hex');
  const originalDigest=digest();
  const base=path.join(temp,'user');
  const probeEnv={...process.env,APPDATA:base,LOCALAPPDATA:path.join(temp,'unused-local'),HOME:base,XDG_DATA_HOME:base};
  const user=path.join(base,...(process.platform==='win32'?['Loom']:process.platform==='darwin'?['Library','Application Support','Loom']:['loom']),'language-packs');
  for(const [name,text] of [['new.json','{"a": true}'],['new.md','# Heading\n\n*emphasis*\n'],['new.downloaded','fn hello() {}'],['new.rs','fn normal() {}']])fs.writeFileSync(path.join(temp,name),text);
  const probe=name=>JSON.parse(execFileSync(binary,[path.join(temp,name)],{env:probeEnv,cwd:temp,encoding:'utf8'}));
  for(const name of ['new.json','new.md','new.downloaded'])assert.deepEqual(probe(name),{language:'plaintext',colored:0,modules:[],wasm_modules:[],errors:[]});
  const portable=path.join(temp,'language-packs');
  fs.mkdirSync(portable,{recursive:true});
  for(const id of ['json','markdown'])fs.cpSync(collections[id],path.join(portable,id),{recursive:true});
  const custom=path.join(portable,'downloaded');
  fs.cpSync(path.join(legacyPackages,'rust'),custom,{recursive:true});
  const original=JSON.parse(fs.readFileSync(path.join(custom,'package.json'),'utf8'));
  fs.renameSync(path.join(custom,'grammars/rust'),path.join(custom,'grammars/downloaded'));
  fs.writeFileSync(path.join(custom,'package.json'),JSON.stringify({schema:1,id:'downloaded',languages:[{
    name:'downloaded',display_name:'Downloaded',badge:'D',variants:[{id:'downloaded',grammar:'downloaded'}],files:[{extensions:['downloaded'],variant:'downloaded'}]
  }],grammars:{downloaded:{...original.grammars.rust,symbol:'tree_sitter_rust'}}}));
  const installed={};
  for(const [name,language] of [['new.json','json'],['new.md','markdown'],['new.downloaded','downloaded']]){
    const result=probe(name);assert.equal(result.language,language);assert.ok(result.colored>0);assert.equal(result.modules.length,language==='markdown'?2:1);assert.deepEqual(result.errors,[]);installed[name]=result;
  }
  assert.equal(probe('new.rs').language,'plaintext');
  fs.mkdirSync(path.dirname(user),{recursive:true});
  fs.renameSync(portable,user);
  for(const [name,language] of [['new.json','json'],['new.md','markdown'],['new.downloaded','downloaded']]){
    const result=probe(name);assert.equal(result.language,language);assert.ok(result.colored>0);assert.deepEqual(result.errors,[]);
  }
  let extra=null;
  if(extraPackage){
    const standardPath=path.join(extraPackage,'tree-sitter.json');
    const isStandard=fs.existsSync(standardPath);
    const manifest=JSON.parse(fs.readFileSync(isStandard?standardPath:path.join(extraPackage,'package.json'),'utf8'));
    const id=isStandard?manifest.grammars[0].name:manifest.id;
    const language=isStandard?manifest.grammars[0].name:manifest.languages[0].name;
    assert.match(id,/^[a-z][a-z0-9_]*$/);
    const name='unseen'+path.extname(sample);
    fs.copyFileSync(sample,path.join(temp,name));
    assert.deepEqual(probe(name),{language:'plaintext',colored:0,modules:[],wasm_modules:[],errors:[]});
    assert.equal(fs.existsSync(path.join(user,id)),false);
    fs.cpSync(extraPackage,path.join(user,id),{recursive:true});
    extra=probe(name);
    assert.equal(extra.language,language);
    assert.ok(extra.colored>0);assert.ok(extra.modules.length>0);assert.deepEqual(extra.errors,[]);
  }
  assert.equal(digest(),originalDigest); // Same executable before and after resource installation.
  if(fs.existsSync(path.join(user,'json/tree-sitter.json'))) fs.renameSync(path.join(user,'json'),path.join(base,'removed-json'));
  else { const jsonPath=path.join(user,'json/package.json'),disabled=JSON.parse(fs.readFileSync(jsonPath));disabled.enabled=false;fs.writeFileSync(jsonPath,JSON.stringify(disabled)); }
  assert.equal(probe('new.json').language,'plaintext');
  console.log(JSON.stringify({noPackages:'plaintext',installed,extra,portableAndUser:true,unchangedExecutable:true,restartDisable:'plaintext'}));
} finally {
  assert.equal(path.dirname(temp),os.tmpdir());
  fs.rmSync(temp,{recursive:true,force:true});
}
