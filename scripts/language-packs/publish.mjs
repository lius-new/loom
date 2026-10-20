import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {ROOT, WORK, CATALOG, assert, HASH, readToml, loadRecipes, inputDigest, hash, fileHash,
  checkArtifacts, makeCatalog, validateCatalog, writeAtomic, toml, run, options, main} from './lib.mjs';

export function preparePublication(args) {
  const dist = path.join(args.work || WORK,'dist'), reportFile = path.join(dist,'build-report.json');
  const report = JSON.parse(fs.readFileSync(reportFile,'utf8'));
  assert(report.schema === 1 && report.complete && !report.failures.length, 'Build every selected package successfully before preparing publication');
  assert(report.input === inputDigest(loadRecipes(args.recipes)), 'Build inputs changed; rebuild before publication');
  const catalogFile = args.catalog || CATALOG;
  const current = validateCatalog(readToml(catalogFile));
  const repository = args.repository || current.distribution.repository;
  assert(current.distribution.repository === repository, 'Changing the release repository would invalidate existing package URLs');
  const cleanReport = {...report, packages:Object.fromEntries(Object.entries(report.packages).map(([id,{validation,...record}])=>[id,record]))};
  const added = makeCatalog(cleanReport,{repository,release:args.release});
  assert(report.selected.length === Object.keys(added.packages).length && report.selected.every(id=>added.packages[id]), 'Build report does not match selected packages');
  checkArtifacts(added,dist);
  for (const [id,pkg] of Object.entries(added.packages)) {
    const old = current.packages[id];
    assert(!old || old.release !== pkg.release || old.sha256 === pkg.sha256, `Published ${id} is different; choose a new resource tag`);
  }
  const candidate = {...current,packages:{...current.packages,...added.packages}};
  const candidateFile = path.join(dist,'language-packs.toml');
  writeAtomic(candidateFile,toml(candidate));
  const sums = Object.values(added.packages).map(pkg=>`${pkg.sha256}  ${pkg.asset}`).sort().join('\n')+'\n';
  writeAtomic(path.join(dist,'SHA256SUMS'),sums);
  const assets = [...Object.values(added.packages).map(p=>p.asset),'language-packs.toml','SHA256SUMS','build-report.json'];
  const plan = {schema:1, repository, release:args.release, input:report.input,
    base:fileHash(catalogFile), packages:report.selected, assets:Object.fromEntries(assets.map(name=>[name,fileHash(path.join(dist,name))]))};
  writeAtomic(path.join(dist,'publication.json'),JSON.stringify(plan,null,2)+'\n');
  return {candidate,plan};
}

export function verifyPublication(args) {
  const dist = path.join(args.work || WORK,'dist'), plan = JSON.parse(fs.readFileSync(path.join(dist,'publication.json'),'utf8'));
  const catalog = validateCatalog(readToml(path.join(dist,'language-packs.toml')));
  assert(plan.schema === 1 && /^language-packs-[\w.-]+$/.test(plan.release), 'Invalid publication plan');
  assert(plan.repository === catalog.distribution.repository, 'Publication repository mismatch');
  if (args.release) assert(args.release === plan.release,'Publication tag mismatch');
  if (args.repository) assert(args.repository === plan.repository,'Publication repository mismatch');
  assert(plan.input === inputDigest(loadRecipes(args.recipes)), 'Build inputs changed; rebuild before publication');
  const subset = {...catalog,packages:{}};
  for (const id of plan.packages) {
    assert(catalog.packages[id]?.release === plan.release, `Candidate package does not match publication: ${id}`);
    subset.packages[id] = catalog.packages[id];
  }
  checkArtifacts(subset,dist);
  const report=JSON.parse(fs.readFileSync(path.join(dist,'build-report.json'),'utf8'));
  assert(report.complete && report.input===plan.input && JSON.stringify(report.selected)===JSON.stringify(plan.packages),'Publication build report mismatch');
  const expected = [...plan.packages.map(id=>`${id}.tar.gz`),'language-packs.toml','SHA256SUMS','build-report.json'].sort();
  assert(JSON.stringify(Object.keys(plan.assets).sort()) === JSON.stringify(expected),'Unexpected publication assets');
  for (const [name,digest] of Object.entries(plan.assets)) {
    assert(HASH.test(digest) && fileHash(path.join(dist,name)) === digest, `Publication file changed: ${name}`);
  }
  return {dist,plan,catalog};
}

export class GitHubReleases {
  constructor(repository) { this.repository=repository; }
  view(tag) {
    try {return JSON.parse(run('gh',['api',`repos/${this.repository}/releases/tags/${tag}`]));}
    catch(error) {if (/HTTP 404|Not Found \(HTTP 404\)/.test(String(error.stderr || error.message))) return null; throw error;}
  }
  create(tag,commit,notes) {
    run('gh',['release','create',tag,'--repo',this.repository,'--target',commit,'--draft','--latest=false','--title',`Loom language packages — ${tag}`,'--notes-file',notes]);
    // GitHub's tags endpoint does not expose draft releases. Use release view.
    return this.draft(tag);
  }
  draft(tag) {
    const data=JSON.parse(run('gh',['release','view',tag,'--repo',this.repository,'--json','databaseId,tagName,isDraft,assets']));
    return {id:data.databaseId,tag_name:data.tagName,draft:data.isDraft,assets:data.assets};
  }
  lookup(tag) {
    // Authenticated gh release view includes drafts, while the tag API may not.
    try { return this.draft(tag); }
    catch(error) {
      const message=String(error.stderr || error.message);
      if (/release not found|HTTP 404|Not Found/i.test(message)) return null;
      throw error;
    }
  }
  assetDigest(tag,asset,temporary) {
    const digest=asset.digest?.replace(/^sha256:/,'');
    if (HASH.test(digest)) return digest;
    fs.mkdirSync(temporary,{recursive:true});
    const directory=fs.mkdtempSync(path.join(temporary,'asset-'));
    run('gh',['release','download',tag,'--repo',this.repository,'--pattern',asset.name,'--dir',directory]);
    return fileHash(path.join(directory,asset.name));
  }
  upload(tag,file) {run('gh',['release','upload',tag,file,'--repo',this.repository]);}
  publish(tag) {run('gh',['release','edit',tag,'--repo',this.repository,'--draft=false','--latest=false']);}
}

export async function publishPrepared(args, github, {commit, requireClean = true} = {}) {
  const {dist,plan,catalog}=verifyPublication(args), catalogFile=args.catalog || CATALOG;
  assert(fileHash(catalogFile) === plan.base || fileHash(catalogFile) === plan.assets['language-packs.toml'], 'Installed publication manifest changed since preparation');
  if(requireClean) assert(run('git',['status','--porcelain']).length === 0,'Commit the recipes, tools and local resources before publishing');
  commit ||= run('git',['rev-parse','HEAD']).trim();
  assert(/^[a-f0-9]{40}$/.test(commit),'Pin publication to a concrete repository commit');
  github ||= new GitHubReleases(plan.repository);
  const notes=path.join(dist,'release-notes.md');
  writeAtomic(notes,`Language resources for Loom.\n\n${plan.packages.map(id=>`- ${id}`).join('\n')}\n\nSource commit: ${commit}\n`);
  let release=github.lookup(plan.release);
  if(!release) release=github.create(plan.release,commit,notes);
  assert(release.tag_name === plan.release,'GitHub returned a different resource tag');
  const downloads=path.join(dist,'verify-'+plan.release);
  for (const [name,digest] of Object.entries(plan.assets)) {
    const existing=release.assets.find(asset=>asset.name===name);
    if(existing) {
      const temporary=path.join(downloads,name.replace(/[^\w.-]/g,'_'));
      assert(github.assetDigest(plan.release,existing,temporary) === digest,`Remote asset differs: ${name}; existing assets are never overwritten`);
    } else {
      assert(release.draft,`Published release is missing ${name}; choose a new resource tag`);
      github.upload(plan.release,path.join(dist,name));
    }
  }
  // Verify the server's complete asset set before publishing or promoting locally.
  release=github.lookup(plan.release);
  for(const [name,digest]of Object.entries(plan.assets)) {
    const asset=release.assets.find(asset=>asset.name===name); assert(asset,`Upload missing on server: ${name}`);
    const temporary=path.join(downloads,'final-'+name.replace(/[^\w.-]/g,'_'));
    assert(github.assetDigest(plan.release,asset,temporary)===digest,`Uploaded asset digest differs: ${name}`);
  }
  if(args['draft-only']) return {published:false,release:plan.release};
  if(release.draft) github.publish(plan.release);
  assert(github.lookup(plan.release)?.draft === false,'Release publication was not confirmed');
  assert(fileHash(catalogFile) === plan.base || fileHash(catalogFile) === plan.assets['language-packs.toml'], 'Publication manifest changed during upload');
  writeAtomic(catalogFile,fs.readFileSync(path.join(dist,'language-packs.toml')));
  return {published:true,release:plan.release,packages:plan.packages};
}

if(process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main(async()=>{
  const [command,...argv]=process.argv.slice(2),args=options(argv);
  if(args.help||!command||command==='--help'){console.log('node scripts/language-packs/publish.mjs prepare --release language-packs-001\nnode scripts/language-packs/publish.mjs verify\nnode scripts/language-packs/publish.mjs publish [--draft-only]');return;}
  if(command==='prepare') console.log(JSON.stringify(preparePublication(args).plan,null,2));
  else if(command==='verify') console.log(`Verified ${verifyPublication(args).plan.release}; no remote changes`);
  else if(command==='publish') console.log(JSON.stringify(await publishPrepared(args)));
  else throw Error(`Unknown publication command: ${command}`);
});
