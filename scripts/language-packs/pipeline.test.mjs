import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {stringify} from 'smol-toml';
import {ROOT, WORK, loadRecipes, validateRecipes, validateCatalog, hash, fileHash, inputDigest, toml,
  createArchive, extractArchive, fetchPinned, rewriteCaptures, applyQueryPatch, downloadUrl} from './lib.mjs';
import {preparePublication, verifyPublication, publishPrepared} from './publish.mjs';

function fixture(t) {
  const parent=path.join(WORK,'tests');fs.mkdirSync(parent,{recursive:true});
  const work=fs.mkdtempSync(path.join(parent,'case-'));
  t.after(()=>{assert.ok(work.startsWith(parent+path.sep));fs.rmSync(work,{recursive:true,force:true});});
  const source=path.join(work,'source');fs.mkdirSync(source);fs.writeFileSync(path.join(source,'LICENSE'),'license');fs.writeFileSync(path.join(source,'sample'),'int main() { return 0; }');
  const rel=path.relative(ROOT,source).replaceAll('\\','/');
  const tool={url:'https://example.test/tool',sha256:'a'.repeat(64),format:'gzip'};
  const recipes={schema:1,tools:{'cli-version':'0.27.1','generate-abi':15,cli:{'linux-x64':tool},wasi:{'linux-x64':{...tool,format:'tar.gz'}},binaryen:{'linux-x64':{...tool,format:'tar.gz'}}},
    sources:{c:{directory:rel,licenses:['LICENSE']}},packages:{c:{name:'C',grammars:{c:{source:'c',path:'.','file-types':['c'],sample:rel+'/sample',highlights:[{path:'query.scm'}],injections:[]}}}}};
  const recipesFile=path.join(work,'build.toml');fs.writeFileSync(recipesFile,stringify(recipes));
  const catalog=path.join(work,'catalog.toml');fs.writeFileSync(catalog,toml({schema:1,distribution:{repository:'owner/loom'},packages:{}}));
  const dist=path.join(work,'dist');fs.mkdirSync(dist);const archive=path.join(dist,'c.tar.gz');fs.writeFileSync(archive,'verified archive');
  const record={name:'C',asset:'c.tar.gz',sha256:fileHash(archive),size:fs.statSync(archive).size,grammars:{c:{'file-types':['c']}}};
  const report={schema:1,complete:true,input:inputDigest(recipes),selected:['c'],packages:{c:record},failures:[]};
  fs.writeFileSync(path.join(dist,'build-report.json'),JSON.stringify(report));
  return {work,source,recipes,recipesFile,catalog,dist,record,report,args:{work,recipes:recipesFile,catalog,repository:'owner/loom',release:'language-packs-001'}};
}
function githubFor(plan,{failUpload=false,failPublish=false}={}) {
  let release=null; const calls=[];
  return {calls,lookup:()=>release,create:(tag)=>{calls.push('create');return release={tag_name:tag,draft:true,assets:[]};},
    assetDigest:(_tag,asset)=>asset.digest,
    upload:(_tag,file)=>{calls.push(path.basename(file));if(failUpload)throw Error('upload failed');release.assets.push({name:path.basename(file),digest:plan.assets[path.basename(file)]});},
    publish:()=>{if(failPublish)throw Error('publish failed');calls.push('publish');release.draft=false;}};
}

test('the production recipe set covers all 50 packages and keeps publishing separate',()=>{
  const data=loadRecipes();assert.equal(Object.keys(data.packages).length,50);
  assert.ok(data.packages.typescript.grammars.tsx);assert.ok(data.packages.markdown.grammars.markdown_inline);
  assert.equal(data.distribution,undefined);for(const pkg of Object.values(data.packages)){assert.equal(pkg.sha256,undefined);assert.equal(pkg.release,undefined);}
  assert.throws(()=>validateCatalog({schema:1,distribution:{repository:'a/b'},packages:{},sources:{}}),/Build recipes/);
});
test('recipes reject floating revisions, traversals and conflicting file types',t=>{
  const f=fixture(t),data=structuredClone(f.recipes);
  data.sources.c={repository:'https://github.com/a/b',revision:'main',sha256:'a'.repeat(64),licenses:['LICENSE']};
  assert.throws(()=>validateRecipes(data),/complete commit/);
  data.sources=f.recipes.sources;data.packages.c.grammars.c.path='../escape';assert.throws(()=>validateRecipes(data),/Invalid/);
  data.packages.c.grammars.c.path='.';data.packages.other=structuredClone(data.packages.c);data.packages.other.grammars={other:data.packages.other.grammars.c};
  assert.throws(()=>validateRecipes(data),/Conflicting file type/);
});
test('archives are reproducible and use a single package directory',async t=>{
  const f=fixture(t),dir=path.join(f.work,'c');fs.mkdirSync(dir);fs.writeFileSync(path.join(dir,'a'),'content');
  const a=path.join(f.work,'a.tar.gz'),b=path.join(f.work,'b.tar.gz');await createArchive(dir,a);
  fs.utimesSync(path.join(dir,'a'),new Date(),new Date());await createArchive(dir,b);assert.equal(fileHash(a),fileHash(b));
  const out=path.join(f.work,'out');await extractArchive(a,out);assert.equal(fs.readFileSync(path.join(out,'c/a'),'utf8'),'content');
});
test('cached downloads are verified and offline inputs cannot silently change',async t=>{
  const f=fixture(t),file=path.join(f.work,'download');fs.writeFileSync(file,'good');
  assert.equal(await fetchPinned('https://example.test/input',hash('good'),file,{offline:true}),file);
  fs.writeFileSync(file,'bad');await assert.rejects(fetchPinned('https://example.test/input',hash('good'),file,{offline:true}),/corrupt offline/);
  await assert.rejects(fetchPinned('https://example.test/input',hash('good'),file,{fetcher:async()=>new Response('wrong')}),/SHA-256/);
  assert.equal(fs.readFileSync(file,'utf8'),'bad');
});
test('capture adaptations affect tokens, not query strings or comments',()=>{
  assert.equal(rewriteCaptures('; @boolean\n((node) @boolean (#eq? @boolean "@boolean"))',{boolean:'constant.builtin'}),'; @boolean\n((node) @constant.builtin (#eq? @constant.builtin "@boolean"))');
});
test('query patches cannot write outside the staged query directory',t=>{
  const f=fixture(t),patch=path.join(f.work,'escape.patch');fs.writeFileSync(patch,'--- a/../escape\n+++ b/../escape\n@@ -1 +1 @@\n-a\n+b\n');
  assert.throws(()=>applyQueryPatch(f.work,patch),/Invalid|Patches/);
});
test('preparing publication writes a candidate without changing the shipped manifest',t=>{
  const f=fixture(t),before=fs.readFileSync(f.catalog,'utf8');const {candidate}=preparePublication(f.args);
  assert.equal(fs.readFileSync(f.catalog,'utf8'),before);assert.equal(candidate.sources,undefined);assert.equal(candidate.packages.c.validation,undefined);
  assert.equal(downloadUrl(candidate,'c'),'https://github.com/owner/loom/releases/download/language-packs-001/c.tar.gz');
  verifyPublication(f.args);fs.appendFileSync(path.join(f.dist,'c.tar.gz'),'tamper');assert.throws(()=>verifyPublication(f.args),/mismatch/);
});
test('incomplete builds and changed local inputs cannot be prepared or published',t=>{
  const f=fixture(t);f.report.complete=false;fs.writeFileSync(path.join(f.dist,'build-report.json'),JSON.stringify(f.report));assert.throws(()=>preparePublication(f.args),/successfully/);
  f.report.complete=true;fs.writeFileSync(path.join(f.dist,'build-report.json'),JSON.stringify(f.report));preparePublication(f.args);
  fs.appendFileSync(path.join(f.source,'sample'),'changed');assert.throws(()=>verifyPublication(f.args),/inputs changed/);
});
test('upload or publish failure leaves the application manifest untouched',async t=>{
  for(const failure of [{failUpload:true},{failPublish:true}]){
    const f=fixture(t),before=fileHash(f.catalog),{plan}=preparePublication(f.args),github=githubFor(plan,failure);
    await assert.rejects(publishPrepared(f.args,github,{requireClean:false,commit:'a'.repeat(40)}),/failed/);assert.equal(fileHash(f.catalog),before);
  }
});
test('draft publication is resumable and promotion happens only after confirmation',async t=>{
  const f=fixture(t),before=fileHash(f.catalog),{plan}=preparePublication(f.args),github=githubFor(plan);
  const draft=await publishPrepared({...f.args,'draft-only':true},github,{requireClean:false,commit:'a'.repeat(40)});
  assert.equal(draft.published,false);assert.equal(fileHash(f.catalog),before);const uploads=github.calls.length;
  const result=await publishPrepared(f.args,github,{requireClean:false,commit:'a'.repeat(40)});
  assert.equal(result.published,true);assert.equal(github.calls.length,uploads+1);assert.equal(fileHash(f.catalog),plan.assets['language-packs.toml']);
});
test('different remote assets are never overwritten and cannot promote the catalog',async t=>{
  const f=fixture(t),before=fileHash(f.catalog),{plan}=preparePublication(f.args),github=githubFor(plan);
  await publishPrepared({...f.args,'draft-only':true},github,{requireClean:false,commit:'a'.repeat(40)});
  github.lookup().assets[0].digest='f'.repeat(64);const calls=github.calls.length;
  await assert.rejects(publishPrepared(f.args,github,{requireClean:false,commit:'a'.repeat(40)}),/never overwritten/);
  assert.equal(github.calls.length,calls);assert.equal(fileHash(f.catalog),before);
});
