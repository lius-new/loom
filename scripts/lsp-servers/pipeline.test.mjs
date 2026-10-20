// Tests for the language server pipeline. They use fake archives and a fake
// release host: nothing here downloads, runs npm or Go, or touches the network.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import zlib from 'node:zlib';
import {tar, stringifyToml} from '../language-packs/lib.mjs';
import {loadRecipes, validateRecipes, loadPins, validatePins, validateLocks, validateCatalog, makeCatalog, catalogToml, assetUrl,
  assemblePack, extractZip, inputDigest, downloadUrl, fileHash, hash, serverEntry, readToml, packRecord, assetName, createArchive, packPlatforms, PLATFORMS} from './lib.mjs';
import {preparePublication, verifyPublication, publishPrepared} from './publish.mjs';
import {expand, expandValue, resolveCommand, handshake} from './validate.mjs';

const temporary = t => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'loom-lsp-test-'));
  // A server killed a moment ago can still hold its working directory on Windows.
  t.after(() => fs.rmSync(directory, {recursive: true, force: true, maxRetries: 20, retryDelay: 100}));
  return directory;
};

function makeZip(entries) {
  const chunks = [], central = []; let offset = 0;
  for (const {name, data = '', deflate = true, mode = 0o644, link = false} of entries) {
    const content = Buffer.from(data), body = deflate ? zlib.deflateRawSync(content) : content, method = deflate ? 8 : 0;
    const crc = zlib.crc32(content), nameBuffer = Buffer.from(name);
    const header = Buffer.alloc(30);
    header.writeUInt32LE(0x04034b50, 0); header.writeUInt16LE(20, 4); header.writeUInt16LE(method, 8); header.writeUInt32LE(crc, 14);
    header.writeUInt32LE(body.length, 18); header.writeUInt32LE(content.length, 22); header.writeUInt16LE(nameBuffer.length, 26);
    chunks.push(header, nameBuffer, body);
    const entry = Buffer.alloc(46);
    entry.writeUInt32LE(0x02014b50, 0); entry.writeUInt16LE(0x031e, 4); entry.writeUInt16LE(20, 6); entry.writeUInt16LE(method, 10);
    entry.writeUInt32LE(crc, 16); entry.writeUInt32LE(body.length, 20); entry.writeUInt32LE(content.length, 24); entry.writeUInt16LE(nameBuffer.length, 28);
    entry.writeUInt32LE(((link ? 0o120777 : (name.endsWith('/') ? 0o040755 : 0o100000 | mode)) << 16) >>> 0, 38); entry.writeUInt32LE(offset, 42);
    central.push(entry, nameBuffer); offset += 30 + nameBuffer.length + body.length;
  }
  const directory = Buffer.concat(central), end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0); end.writeUInt16LE(entries.length, 8); end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(directory.length, 12); end.writeUInt32LE(offset, 16);
  return Buffer.concat([...chunks, directory, end]);
}

// ---------------------------------------------------------------- the real recipes

test('the production recipes define every server of the design with a runtime-relative command', () => {
  const recipes = loadRecipes();
  const expected = ['bash-language-server', 'biome', 'clangd', 'gopls', 'lua-language-server', 'marksman', 'pyright', 'ruff', 'rust-analyzer', 'taplo',
    'typescript-language-server', 'vscode-css-language-server', 'vscode-eslint-language-server', 'vscode-html-language-server', 'vscode-json-language-server',
    'yaml-language-server', 'zls'];
  assert.deepEqual(Object.keys(recipes.servers).sort(), expected);
  for (const [id, server] of Object.entries(recipes.servers)) {
    assert.match(server.command, /^\$\{(pack|runtime\.node)\}\//, id);
    if (server.command.startsWith('${runtime.')) assert.deepEqual(recipes.packs[server.pack].requires, ['node@22'], id);
  }
  assert.equal(recipes.packs.node.kind, 'archive');
  assert.equal(recipes.packs.gopls.kind, 'go');
  for (const platform of PLATFORMS) assert.ok(recipes.packs.node.platforms[platform], `node for ${platform}`);
});

test('every download is pinned and every npm pack is locked, with no hand-written catalog entries', () => {
  const recipes = loadRecipes();
  validatePins(recipes, loadPins());
  validateLocks(recipes);
  // The committed catalog is generated: it holds servers only after a release.
  validateCatalog(readToml(new URL('../../assets/lsp-servers/lsp-servers.toml', import.meta.url)));
});

test('recipes refuse programs found on PATH, unrequired runtimes and floating versions', () => {
  const base = loadRecipes();
  const change = edit => { const data = structuredClone(base); edit(data); return data; };
  assert.throws(() => validateRecipes(change(d => { d.servers['rust-analyzer'].command = 'rust-analyzer'; })), /never names a program found on PATH/);
  assert.throws(() => validateRecipes(change(d => { d.servers['rust-analyzer'].command = '${runtime.node}/node'; })), /does not require/);
  assert.throws(() => validateRecipes(change(d => { d.packs.pyright.packages = ['pyright@^1.1.0']; })), /exact npm packages/);
  assert.throws(() => validateRecipes(change(d => { d.packs.pyright.requires = ['node@18']; })), /no pack provides/);
  assert.throws(() => validateRecipes(change(d => { d.servers.pyright.pack = 'missing'; })), /unknown pack/);
  assert.throws(() => validateRecipes(change(d => { d.packs.gopls['module-version'] = 'master'; })), /Pin the Go toolchain/);
  assert.throws(() => validateRecipes(change(d => { d.packs.taplo.platforms['linux-x86_64'].format = 'rar'; })), /Unknown format/);
  assert.throws(() => validateRecipes(change(d => { delete d.packs.gopls.licenses; })), /names its license files/);
  assert.throws(() => validateRecipes(change(d => { d.packs.pyright.licenses = [{name: 'LICENSE', url: 'https://example.test/LICENSE'}]; })), /node_modules/);
  assert.throws(() => validateRecipes(change(d => { d.servers.taplo.languages.push({language: 'toml', language_id: 'x'}); })), /bound twice/);
});

test('a stale or missing pin stops the build', () => {
  const recipes = loadRecipes(), pins = structuredClone(loadPins());
  pins.packs.taplo.platforms['linux-x86_64'].url += '.old';
  assert.throws(() => validatePins(recipes, pins), /Stale pin/);
  delete pins.packs.taplo.platforms['linux-x86_64'];
  assert.throws(() => validatePins(recipes, pins), /Missing pin/);
  const extra = structuredClone(loadPins()); extra.packs.taplo.platforms['linux-aarch64'].sha256 = 'abc';
  assert.throws(() => validatePins(recipes, extra), /Invalid pin/);
});

// ---------------------------------------------------------------- extraction

test('zip extraction keeps files and modes and refuses links and escapes', t => {
  const directory = temporary(t), archive = path.join(directory, 'a.zip');
  fs.writeFileSync(archive, makeZip([
    {name: 'top/'}, {name: 'top/bin/run', data: 'binary', mode: 0o755}, {name: 'top/stored.txt', data: 'plain', deflate: false}, {name: 'top/link', data: 'run', link: true},
  ]));
  extractZip(archive, path.join(directory, 'out'));
  assert.equal(fs.readFileSync(path.join(directory, 'out/top/bin/run'), 'utf8'), 'binary');
  assert.equal(fs.readFileSync(path.join(directory, 'out/top/stored.txt'), 'utf8'), 'plain');
  assert.ok(!fs.existsSync(path.join(directory, 'out/top/link')), 'links are never extracted');
  if (process.platform !== 'win32') assert.ok(fs.statSync(path.join(directory, 'out/top/bin/run')).mode & 0o111);
  fs.writeFileSync(archive, makeZip([{name: '../escape', data: 'x'}]));
  assert.throws(() => extractZip(archive, path.join(directory, 'again')), /Invalid archive entry/);
  fs.writeFileSync(archive, Buffer.from('not a zip'));
  assert.throws(() => extractZip(archive, path.join(directory, 'third')), /Not a zip/);
});

// ---------------------------------------------------------------- assembly

function pinned(directory, files) {
  // Pre-populates the download cache, so a pinned input is found without any network.
  const pins = {schema: 1, packs: {}};
  const downloads = path.join(directory, 'work', 'downloads');
  fs.mkdirSync(downloads, {recursive: true});
  for (const [key, content] of Object.entries(files)) { const sha = hash(content); fs.writeFileSync(path.join(downloads, sha), content); files[key] = {sha256: sha, size: content.length}; }
  return {pins, files};
}

async function demoPack(t) {
  const directory = temporary(t), tarSource = path.join(directory, 'demo-1.2.3');
  fs.mkdirSync(path.join(tarSource, 'lib'), {recursive: true});
  fs.writeFileSync(path.join(tarSource, 'demo'), 'unix binary'); fs.writeFileSync(path.join(tarSource, 'lib/data'), 'data');
  const tarFile = path.join(directory, 'demo.tar.gz');
  await tar.c({file: tarFile, cwd: directory, gzip: true, portable: true}, ['demo-1.2.3']);
  const files = {
    tar: fs.readFileSync(tarFile),
    zip: makeZip([{name: 'README', data: 'notes'}, {name: 'bin/demo.exe', data: 'windows binary'}, {name: 'bin/extra.dll', data: 'library'}]),
    gz: zlib.gzipSync('mac binary'), license: Buffer.from('MIT license text'),
  };
  const { files: pins } = pinned(directory, files);
  const recipes = {schema: 1, packs: {demo: {kind: 'archive', version: '1.2.3', license: 'MIT', upstream: {source: 'github', repository: 'owner/demo', tag: 'v1.2.3'},
    licenses: [{name: 'LICENSE', url: 'https://example.test/LICENSE'}],
    platforms: {
      'linux-x86_64': {asset: 'demo.tar.gz', format: 'tar.gz', executables: ['demo']},
      'windows-x86_64': {asset: 'demo.zip', format: 'zip', executables: ['bin/demo.exe'], include: ['bin/demo.exe']},
      'macos-aarch64': {asset: 'demo.gz', format: 'gz', executables: ['demo']},
    }}},
  servers: {demo: {name: 'Demo', pack: 'demo', command: '${pack}/demo', args: ['--stdio'], root: {markers: ['demo.toml'], fallback: 'workspace'}, languages: [{language: 'rust', language_id: 'rust'}]}}};
  validateRecipes(recipes);
  const pack = recipes.packs.demo;
  const allPins = {schema: 1, packs: {demo: {platforms: {
    'linux-x86_64': {url: assetUrl(pack, 'linux-x86_64'), ...pins.tar},
    'windows-x86_64': {url: assetUrl(pack, 'windows-x86_64'), ...pins.zip},
    'macos-aarch64': {url: assetUrl(pack, 'macos-aarch64'), ...pins.gz},
  }, licenses: {LICENSE: {url: 'https://example.test/LICENSE', ...pins.license}}}}};
  validatePins(recipes, allPins);
  return {directory, recipes, pins: allPins, work: path.join(directory, 'work')};
}

test('packs are assembled from pinned archives of every format without the network', async t => {
  const {recipes, pins, work} = await demoPack(t);
  const options = {work, offline: true};
  const linux = await assemblePack(recipes, pins, 'demo', 'linux-x86_64', options);
  assert.equal(path.basename(linux), 'demo');
  assert.equal(fs.readFileSync(path.join(linux, 'demo'), 'utf8'), 'unix binary', 'the single top-level directory is stripped');
  assert.equal(fs.readFileSync(path.join(linux, 'lib/data'), 'utf8'), 'data');
  assert.equal(fs.readFileSync(path.join(linux, 'licenses/LICENSE'), 'utf8'), 'MIT license text');
  assert.match(fs.readFileSync(path.join(linux, 'PROVENANCE.md'), 'utf8'), /SHA-256: [a-f0-9]{64}/);
  const windows = await assemblePack(recipes, pins, 'demo', 'windows-x86_64', options);
  assert.deepEqual(fs.readdirSync(path.join(windows, 'bin')), ['demo.exe'], 'only the included files are kept');
  const mac = await assemblePack(recipes, pins, 'demo', 'macos-aarch64', options);
  assert.equal(fs.readFileSync(path.join(mac, 'demo'), 'utf8'), 'mac binary');
  if (process.platform !== 'win32') assert.ok(fs.statSync(path.join(mac, 'demo')).mode & 0o111);
});

test('a missing executable, a changed download and an offline gap are errors', async t => {
  const {recipes, pins, work} = await demoPack(t);
  const broken = structuredClone(recipes); broken.packs.demo.platforms['linux-x86_64'].executables = ['missing'];
  await assert.rejects(assemblePack(broken, pins, 'demo', 'linux-x86_64', {work, offline: true}), /has no missing/);
  const wrong = structuredClone(pins); wrong.packs.demo.platforms['linux-x86_64'].sha256 = 'f'.repeat(64);
  await assert.rejects(assemblePack(recipes, wrong, 'demo', 'linux-x86_64', {work, offline: true}), /corrupt offline|Missing/);
  await assert.rejects(assemblePack(recipes, pins, 'demo', 'macos-x86_64', {work, offline: true}), /not built for/);
});

test('archives are reproducible and use one pack directory', async t => {
  const {recipes, pins, work} = await demoPack(t);
  const directory = await assemblePack(recipes, pins, 'demo', 'linux-x86_64', {work, offline: true});
  const a = path.join(work, 'a.tar.gz'), b = path.join(work, 'b.tar.gz');
  await createArchive(directory, a);
  const later = new Date(); fs.utimesSync(path.join(directory, 'demo'), later, later);
  await createArchive(directory, b);
  assert.equal(fileHash(a), fileHash(b));
  const out = path.join(work, 'out'); fs.mkdirSync(out);
  await tar.x({file: a, cwd: out});
  assert.ok(fs.existsSync(path.join(out, 'demo/demo')));
});

// ---------------------------------------------------------------- catalog

function builtReport(recipes, directory, selected = ['demo']) {
  const dist = path.join(directory, 'work', 'dist'); fs.mkdirSync(dist, {recursive: true});
  const report = {schema: 1, complete: true, partial: false, input: null, selected, packs: {}, failures: []};
  for (const id of selected) {
    report.packs[id] = {version: recipes.packs[id].version, platforms: {}};
    for (const platform of Object.keys(recipes.packs[id].platforms)) {
      const archive = path.join(dist, assetName(id, platform)); fs.writeFileSync(archive, `${id} ${platform} archive`);
      report.packs[id].platforms[platform] = packRecord(id, platform, archive);
    }
  }
  return {report, dist};
}

test('the catalog carries each pack once per platform and every server of a published pack', async t => {
  const {recipes, directory} = await demoPack(t);
  const {report} = builtReport(recipes, directory);
  const catalog = makeCatalog(recipes, report, {repository: 'owner/loom', release: 'lsp-servers-001', current: {schema: 1, distribution: {repository: 'owner/loom'}, packs: {}, servers: {}}});
  assert.deepEqual(Object.keys(catalog.packs.demo.platforms).sort(), ['linux-x86_64', 'macos-aarch64', 'windows-x86_64']);
  assert.deepEqual(catalog.servers.demo, serverEntry(recipes.servers.demo));
  assert.equal(downloadUrl(catalog, 'demo', 'linux-x86_64'), 'https://github.com/owner/loom/releases/download/lsp-servers-001/demo-linux-x86_64.tar.gz');
  assert.match(catalogToml(catalog), /^# Generated from verified language server artifacts\./);
  const file = path.join(directory, 'catalog.toml');
  fs.writeFileSync(file, catalogToml(catalog));
  assert.deepEqual(validateCatalog(readToml(file)), catalog, 'the generated file reads back unchanged');
});

test('the catalog rejects missing runtimes, PATH commands, bad digests and mixed platforms', async t => {
  const {recipes, directory} = await demoPack(t);
  const {report} = builtReport(recipes, directory);
  const good = makeCatalog(recipes, report, {repository: 'owner/loom', release: 'lsp-servers-001', current: {schema: 1, distribution: {repository: 'owner/loom'}}});
  const change = edit => { const data = structuredClone(good); edit(data); return data; };
  assert.throws(() => validateCatalog(change(d => { d.servers.demo.command = 'demo'; })), /PATH/);
  assert.throws(() => validateCatalog(change(d => { d.servers.demo.pack = 'other'; })), /unknown pack/);
  assert.throws(() => validateCatalog(change(d => { d.packs.demo.requires = ['node@22']; })), /does not provide/);
  assert.throws(() => validateCatalog(change(d => { d.packs.demo.platforms['linux-x86_64'].sha256 = 'abc'; })), /digest/);
  assert.throws(() => validateCatalog(change(d => { d.packs.demo.platforms['linux-x86_64'].asset = 'x.tar.gz'; })), /release asset/);
  assert.throws(() => validateCatalog(change(d => { d.packs.demo.platforms.any = d.packs.demo.platforms['linux-x86_64']; })), /Invalid platforms/);
  assert.throws(() => validateCatalog(change(d => { d.sources = {}; })), /Build recipes/);
  assert.throws(() => validateCatalog(change(d => { d.packs.demo.platforms['linux-x86_64'].release = 'language-packs-001'; })), /release asset/);
});

test('an incomplete or partial build cannot become a catalog', async t => {
  const {recipes, directory} = await demoPack(t);
  const {report} = builtReport(recipes, directory);
  const current = {schema: 1, distribution: {repository: 'owner/loom'}};
  assert.throws(() => makeCatalog(recipes, {...report, complete: false}, {repository: 'owner/loom', release: 'lsp-servers-001', current}), /incomplete/);
  delete report.packs.demo.platforms['linux-x86_64'];
  assert.throws(() => makeCatalog(recipes, report, {repository: 'owner/loom', release: 'lsp-servers-001', current}), /every platform/);
});

// ---------------------------------------------------------------- publication

async function publication(t) {
  const {recipes, pins, directory} = await demoPack(t);
  const recipesFile = path.join(directory, 'build.toml'), pinsFile = path.join(directory, 'pins.toml'), catalogFile = path.join(directory, 'catalog.toml');
  fs.writeFileSync(recipesFile, stringifyToml(recipes)); fs.writeFileSync(pinsFile, stringifyToml(pins));
  fs.writeFileSync(catalogFile, catalogToml({schema: 1, distribution: {repository: 'owner/loom'}}));
  const {report, dist} = builtReport(recipes, directory);
  report.input = inputDigest(loadRecipes(recipesFile), loadPinsFile(pinsFile));
  fs.writeFileSync(path.join(dist, 'build-report.json'), JSON.stringify(report));
  return {directory, dist, catalogFile, recipes, args: {work: path.join(directory, 'work'), recipes: recipesFile, pins: pinsFile, catalog: catalogFile, repository: 'owner/loom', release: 'lsp-servers-001'}};
}
const loadPinsFile = file => loadPins(file);

function githubFor(plan, {failUpload = false, failPublish = false} = {}) {
  let release = null; const calls = [];
  return {calls, lookup: () => release, create: tag => { calls.push('create'); return release = {tag_name: tag, draft: true, assets: []}; },
    assetDigest: (_tag, asset) => asset.digest,
    upload: (_tag, file) => { calls.push(path.basename(file)); if (failUpload) throw Error('upload failed'); release.assets.push({name: path.basename(file), digest: plan.assets[path.basename(file)]}); },
    publish: () => { if (failPublish) throw Error('publish failed'); calls.push('publish'); release.draft = false; }};
}

test('preparing publication writes a candidate without changing the shipped catalog', async t => {
  const f = await publication(t), before = fs.readFileSync(f.catalogFile, 'utf8');
  const {candidate, plan} = preparePublication(f.args);
  assert.equal(fs.readFileSync(f.catalogFile, 'utf8'), before);
  assert.deepEqual(plan.packs, ['demo']);
  assert.ok(plan.assets['lsp-servers.toml'] && plan.assets['demo-linux-x86_64.tar.gz'] && plan.assets.SHA256SUMS);
  assert.equal(downloadUrl(candidate, 'demo', 'windows-x86_64'), 'https://github.com/owner/loom/releases/download/lsp-servers-001/demo-windows-x86_64.tar.gz');
  verifyPublication(f.args);
  fs.appendFileSync(path.join(f.dist, 'demo-linux-x86_64.tar.gz'), 'tamper');
  assert.throws(() => verifyPublication(f.args), /mismatch/);
});

test('incomplete builds and changed inputs cannot be prepared or published', async t => {
  const f = await publication(t);
  const reportFile = path.join(f.dist, 'build-report.json'), report = JSON.parse(fs.readFileSync(reportFile, 'utf8'));
  fs.writeFileSync(reportFile, JSON.stringify({...report, complete: false}));
  assert.throws(() => preparePublication(f.args), /for every platform/);
  fs.writeFileSync(reportFile, JSON.stringify(report));
  preparePublication(f.args);
  const recipes = readToml(f.args.recipes); recipes.packs.demo.version = '9.9.9';
  fs.writeFileSync(f.args.recipes, stringifyToml(recipes));
  assert.throws(() => verifyPublication(f.args), /inputs changed/);
});

test('upload or publish failure leaves the application catalog untouched', async t => {
  for (const failure of [{failUpload: true}, {failPublish: true}]) {
    const f = await publication(t), before = fileHash(f.catalogFile), {plan} = preparePublication(f.args);
    await assert.rejects(publishPrepared(f.args, githubFor(plan, failure), {requireClean: false, commit: 'a'.repeat(40)}), /failed/);
    assert.equal(fileHash(f.catalogFile), before);
  }
});

test('draft publication is resumable and promotion happens only after confirmation', async t => {
  const f = await publication(t), before = fileHash(f.catalogFile), {plan} = preparePublication(f.args), github = githubFor(plan);
  const draft = await publishPrepared({...f.args, 'draft-only': true}, github, {requireClean: false, commit: 'a'.repeat(40)});
  assert.equal(draft.published, false); assert.equal(fileHash(f.catalogFile), before);
  const uploads = github.calls.length;
  const result = await publishPrepared(f.args, github, {requireClean: false, commit: 'a'.repeat(40)});
  assert.equal(result.published, true); assert.equal(github.calls.length, uploads + 1);
  assert.equal(fileHash(f.catalogFile), plan.assets['lsp-servers.toml']);
  assert.ok(readToml(f.catalogFile).servers.demo, 'the released catalog now lists the server');
});

test('different remote assets are never overwritten and cannot promote the catalog', async t => {
  const f = await publication(t), before = fileHash(f.catalogFile), {plan} = preparePublication(f.args), github = githubFor(plan);
  await publishPrepared({...f.args, 'draft-only': true}, github, {requireClean: false, commit: 'a'.repeat(40)});
  github.lookup().assets[0].digest = 'f'.repeat(64);
  const calls = github.calls.length;
  await assert.rejects(publishPrepared(f.args, github, {requireClean: false, commit: 'a'.repeat(40)}), /never overwritten/);
  assert.equal(github.calls.length, calls); assert.equal(fileHash(f.catalogFile), before);
});

test('re-publishing under the same tag with different bytes is refused', async t => {
  const f = await publication(t);
  preparePublication(f.args);
  const released = await publishPrepared(f.args, githubFor(preparePublication(f.args).plan), {requireClean: false, commit: 'a'.repeat(40)});
  assert.equal(released.published, true);
  fs.writeFileSync(path.join(f.dist, 'demo-linux-x86_64.tar.gz'), 'rebuilt differently');
  const report = JSON.parse(fs.readFileSync(path.join(f.dist, 'build-report.json'), 'utf8'));
  report.packs.demo.platforms['linux-x86_64'] = packRecord('demo', 'linux-x86_64', path.join(f.dist, 'demo-linux-x86_64.tar.gz'));
  fs.writeFileSync(path.join(f.dist, 'build-report.json'), JSON.stringify(report));
  assert.throws(() => preparePublication(f.args), /choose a new resource tag/);
});

// ---------------------------------------------------------------- validation helpers

test('placeholders expand in commands, arguments and options only for known runtimes', () => {
  const context = {pack: '/p/pack', runtimes: {node: '/p/node'}};
  assert.equal(expand('${runtime.node}/node', context), '/p/node/node');
  assert.equal(expand('${pack}/a.js', context), '/p/pack/a.js');
  assert.deepEqual(expandValue({tsserver: {path: '${pack}/ts'}, list: ['${pack}/x', 3]}, context), {tsserver: {path: '/p/pack/ts'}, list: ['/p/pack/x', 3]});
  assert.throws(() => expand('${runtime.python}/python', context), /No installed runtime/);
});

test('a program named without an extension is found with .exe on Windows only', t => {
  const directory = temporary(t), program = path.join(directory, 'tool');
  fs.writeFileSync(program + '.exe', '');
  assert.equal(resolveCommand(program, 'win32'), program + '.exe');
  assert.equal(resolveCommand(program, 'linux'), program);
  fs.writeFileSync(program, '');
  assert.equal(resolveCommand(program, 'win32'), program);
});

const FAKE_SERVER = `
let buffer = Buffer.alloc(0);
const send = m => { const b = Buffer.from(JSON.stringify({ jsonrpc: '2.0', ...m })); process.stdout.write('Content-Length: ' + b.length + '\\r\\n\\r\\n'); process.stdout.write(b); };
process.stdin.on('data', chunk => {
  buffer = Buffer.concat([buffer, chunk]);
  for (;;) {
    const end = buffer.indexOf('\\r\\n\\r\\n'); if (end < 0) return;
    const length = Number(/Content-Length: (\\d+)/i.exec(buffer.toString('ascii', 0, end))[1]);
    if (buffer.length < end + 4 + length) return;
    const message = JSON.parse(buffer.subarray(end + 4, end + 4 + length).toString()); buffer = buffer.subarray(end + 4 + length);
    if (message.method === 'initialize') {
      if (process.argv.includes('--fail')) send({ id: message.id, error: { code: -32603, message: 'broken' } });
      else { send({ id: 100, method: 'workspace/configuration', params: { items: [{}] } }); send({ id: message.id, result: { capabilities: { hoverProvider: true, textDocumentSync: 1 } } }); }
    } else if (message.method === 'shutdown') send({ id: message.id, result: null });
    else if (message.method === 'exit') process.exit(0);
  }
});`;

test('the handshake completes against a server and reports its failure otherwise', async t => {
  const directory = temporary(t), script = path.join(directory, 'server.js');
  fs.writeFileSync(script, FAKE_SERVER);
  const capabilities = await handshake({command: process.execPath, args: [script], cwd: directory});
  assert.equal(capabilities.hoverProvider, true);
  await assert.rejects(handshake({command: process.execPath, args: [script, '--fail'], cwd: directory}), /initialize failed/);
  await assert.rejects(handshake({command: path.join(directory, 'missing'), cwd: directory}), /ENOENT|spawn/);
  await assert.rejects(handshake({command: process.execPath, args: ['-e', 'setTimeout(()=>{}, 5000)'], cwd: directory, timeout: 300}), /Timed out/);
});

// ---------------------------------------------------------------- contract with the application

test('the golden catalog is what the recipes generate and what the application must be able to read', () => {
  const recipes = loadRecipes();
  const report = {complete: true, failures: [], selected: Object.keys(recipes.packs).sort(), packs: {}};
  for (const id of report.selected) {
    report.packs[id] = {version: recipes.packs[id].version, platforms: {}};
    for (const platform of packPlatforms(recipes.packs[id])) report.packs[id].platforms[platform] = {asset: assetName(id, platform), sha256: hash(`${id} ${platform}`), size: 1000};
  }
  const current = {schema: 1, distribution: {repository: 'lius-new/loom'}};
  const text = catalogToml(makeCatalog(recipes, report, {repository: 'lius-new/loom', release: 'lsp-servers-fixture', current}));
  const file = new URL('./fixtures/catalog.toml', import.meta.url);
  // The application's catalog reader parses this file in its own tests.
  if (process.env.UPDATE_FIXTURES) fs.writeFileSync(file, text);
  assert.equal(fs.readFileSync(file, 'utf8').replaceAll('\r\n', '\n'), text, 'run with UPDATE_FIXTURES=1 after changing the recipes');
});
