import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import { moduleFile, stageLanguagePacks } from './language-packs.mjs';

function fixture(t, platform) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'loom-packaging-languages-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const profile = path.join(root, 'profile');
  const pack = path.join(profile, 'rust');
  const query = path.join(pack, 'grammars', 'rust');
  fs.mkdirSync(query, { recursive: true });
  fs.writeFileSync(path.join(query, 'highlights.scm'), 'query');
  fs.writeFileSync(path.join(query, 'SOURCE.md'), 'license');
  fs.writeFileSync(path.join(pack, 'package.json'), JSON.stringify({
    schema: 1, id: 'rust', languages: [], servers: [],
    grammars: { rust: { module: 'rust', symbol: 'tree_sitter_rust' } },
  }));
  fs.mkdirSync(path.join(pack, 'modules'));
  fs.writeFileSync(path.join(pack, 'modules', moduleFile('rust', platform)), 'native');
  return { root, profile, pack };
}

test('all platforms stage external modules and only manifest-listed queries', t => {
  for (const platform of ['win32', 'linux', 'darwin']) {
    const { root, profile, pack } = fixture(t, platform);
    fs.mkdirSync(path.join(pack, 'grammars', 'unused'));
    fs.writeFileSync(path.join(pack, 'grammars', 'unused', 'highlights.scm'), 'stale');
    const destination = path.join(root, 'installed');
    const binaries = stageLanguagePacks({ source: profile, destination, platform });
    assert.equal(binaries.length, 1);
    assert.equal(fs.readFileSync(binaries[0], 'utf8'), 'native');
    assert.equal(fs.readFileSync(path.join(destination, 'rust/grammars/rust/highlights.scm'), 'utf8'), 'query');
    assert.equal(fs.existsSync(path.join(destination, 'rust/grammars/unused')), false);
    assert.equal(fs.existsSync(path.join(destination, 'rust/grammars/rust/injections.scm')), false);
  }
});

test('missing modules and malformed paths stop packaging', t => {
  const { root, profile, pack } = fixture(t, 'win32');
  fs.unlinkSync(path.join(pack, 'modules', moduleFile('rust', 'win32')));
  assert.throws(() => stageLanguagePacks({ source: profile, destination: path.join(root, 'installed'), platform: 'win32' }), /Missing prebuilt language module/);
  assert.throws(() => moduleFile('../escaped', 'win32'), /Invalid/);
  const catalog = JSON.parse(fs.readFileSync(path.join(pack, 'package.json')));
  catalog.grammars['../escaped'] = catalog.grammars.rust;
  fs.writeFileSync(path.join(pack, 'package.json'), JSON.stringify(catalog));
  assert.throws(() => stageLanguagePacks({ source: profile, destination: path.join(root, 'invalid'), platform: 'win32' }), /Invalid grammar/);
});

test('packaging requires prebuilt input and copies shared modules once', t => {
  assert.throws(() => stageLanguagePacks({source:'', destination:'unused'}), /prebuilt/);
  const {root,profile,pack}=fixture(t,'win32');
  const manifest=JSON.parse(fs.readFileSync(path.join(pack,'package.json')));
  manifest.grammars.inline=manifest.grammars.rust;
  fs.writeFileSync(path.join(pack,'package.json'),JSON.stringify(manifest));
  fs.cpSync(path.join(pack,'grammars/rust'),path.join(pack,'grammars/inline'),{recursive:true});
  const binaries=stageLanguagePacks({source:profile,destination:path.join(root,'installed'),platform:'win32'});
  assert.equal(binaries.length,1);
  assert.equal(fs.existsSync(path.join(root,'installed/rust/grammars/inline/highlights.scm')),true);
});

test('application assembly directly consumes the complete checked-in Markdown/JSON packages on every platform', t => {
  const {root}=fixture(t,'win32');
  const source=fileURLToPath(new URL('../../../assets/language-packs',import.meta.url));
  for(const platform of ['win32','linux','darwin']) {
    const destination=path.join(root,platform);
    const binaries=stageLanguagePacks({destination,platform});
    assert.equal(binaries.length,3);
    assert.deepEqual(fs.readdirSync(destination).sort(),['json','language-packs.toml','markdown']);
    for(const id of ['json','markdown']) {
      for(const file of fs.readdirSync(path.join(source,id),{recursive:true}).filter(file=>fs.statSync(path.join(source,id,file)).isFile())) {
        assert.deepEqual(fs.readFileSync(path.join(destination,id,file)),fs.readFileSync(path.join(source,id,file)),`${platform}/${id}/${file}`);
      }
    }
    assert.deepEqual(fs.readFileSync(path.join(destination,'language-packs.toml')),fs.readFileSync(path.join(source,'language-packs.toml')));
  }
});

test('standard filenames and default exports need no Loom wrapper or module rename', t => {
  assert.equal(moduleFile('python', 'linux'), 'python.so');
  assert.equal(moduleFile('python', 'darwin'), 'python.dylib');
  assert.equal(moduleFile('python.so', 'darwin'), 'python.so');
  assert.equal(moduleFile('libtree-sitter-python.so', 'linux'), 'libtree-sitter-python.so');
  for (const invalid of ['../python.dll', 'a\\python.dll', 'C:python.dll', 'python.so.exe', 'python..dll']) {
    assert.throws(() => moduleFile(invalid, 'win32'), /Invalid/);
  }
  const {root, profile, pack} = fixture(t, 'win32');
  const manifest = JSON.parse(fs.readFileSync(path.join(pack, 'package.json')));
  manifest.grammars.rust = {module: 'third-party-rust.dll'};
  fs.writeFileSync(path.join(pack, 'package.json'), JSON.stringify(manifest));
  fs.renameSync(path.join(pack, 'modules/rust.dll'), path.join(pack, 'modules/third-party-rust.dll'));
  const binaries = stageLanguagePacks({source: profile, destination: path.join(root, 'installed'), platform: 'win32'});
  assert.equal(path.basename(binaries[0]), 'third-party-rust.dll');
  assert.equal(fs.readFileSync(binaries[0], 'utf8'), 'native');
  assert.throws(() => moduleFile('python.so', 'win32'), /Invalid/);
});

test('upstream WASM names take precedence over native modules on every platform', t => {
  const payload = Buffer.from([0, 97, 115, 109, 1, 0, 0, 0]);
  for (const platform of ['win32', 'linux', 'darwin']) {
    const {root, profile, pack} = fixture(t, platform);
    fs.writeFileSync(path.join(pack, 'modules/tree-sitter-rust.wasm'), payload);
    fs.writeFileSync(path.join(pack, 'modules/rust.wasm'), 'alternate');
    const binaries = stageLanguagePacks({source: profile, destination: path.join(root, 'installed'), platform});
    assert.equal(path.basename(binaries[0]), 'tree-sitter-rust.wasm');
    assert.deepEqual(fs.readFileSync(binaries[0]), payload);
    assert.equal(fs.existsSync(path.join(root, 'installed/rust/modules', moduleFile('rust', platform))), false);
    assert.equal(moduleFile('tree-sitter-rust.wasm', platform), 'tree-sitter-rust.wasm');
  }
});

test('explicit WASM filenames need no native library and remain unmodified', t => {
  const {root, profile, pack} = fixture(t, 'win32');
  const manifest = JSON.parse(fs.readFileSync(path.join(pack, 'package.json')));
  manifest.grammars.rust = {module: 'downloaded.wasm', symbol: 'tree_sitter_rust'};
  fs.writeFileSync(path.join(pack, 'package.json'), JSON.stringify(manifest));
  fs.unlinkSync(path.join(pack, 'modules/rust.dll'));
  fs.writeFileSync(path.join(pack, 'modules/downloaded.wasm'), 'upstream bytes');
  const binaries = stageLanguagePacks({source: profile, destination: path.join(root, 'installed'), platform: 'win32'});
  assert.equal(path.basename(binaries[0]), 'downloaded.wasm');
  assert.equal(fs.readFileSync(binaries[0], 'utf8'), 'upstream bytes');
  assert.throws(() => moduleFile('../rust.wasm', 'win32'), /Invalid/);
  assert.throws(() => moduleFile('rust.wasm.dll', 'win32'), /Invalid/);
});

function standardFixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'loom-upstream-resources-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  const source = path.join(root, 'input'), pack = path.join(source, 'tree-sitter-rust');
  fs.mkdirSync(path.join(pack, 'queries'), {recursive: true});
  const manifest = {grammars: [{name: 'rust', path: '.', 'file-types': ['rs'], highlights: ['queries/base.scm', 'queries/highlights.scm']}], metadata: {license: 'MIT'}, bindings: {rust: true}};
  fs.writeFileSync(path.join(pack, 'tree-sitter.json'), JSON.stringify(manifest, null, 2));
  fs.writeFileSync(path.join(pack, 'tree-sitter-rust.wasm'), 'unchanged upstream WASM');
  for (const file of ['base.scm', 'highlights.scm']) fs.writeFileSync(path.join(pack, 'queries', file), file);
  return {root, source, pack, manifest};
}

test('standard manifests and resource layouts stage on all platforms without Loom files', t => {
  const {root, source, pack} = standardFixture(t);
  fs.writeFileSync(path.join(pack, 'LICENSE'), 'upstream license');
  fs.writeFileSync(path.join(pack, 'package.json'), 'invalid legacy manifest must not be read');
  for (const platform of ['win32', 'linux', 'darwin']) {
    const destination = path.join(root, platform);
    const files = stageLanguagePacks({source, destination, platform});
    assert.equal(files.length, 1);
    assert.equal(fs.readFileSync(files[0], 'utf8'), 'unchanged upstream WASM');
    for (const file of ['tree-sitter.json', 'queries/base.scm', 'queries/highlights.scm', 'LICENSE']) assert.deepEqual(fs.readFileSync(path.join(destination, 'tree-sitter-rust', file)), fs.readFileSync(path.join(pack, file)));
    for (const file of ['package.json', 'SOURCE.md', 'modules', 'grammars']) assert.equal(fs.existsSync(path.join(destination, 'tree-sitter-rust', file)), false);
  }
});

test('standard multi-grammar resources preserve root-relative query arrays and nested modules', t => {
  const {root, source, pack, manifest} = standardFixture(t);
  manifest.grammars.push({name: 'inline', path: 'nested/inline', 'file-types': null, highlights: 'queries/base.scm'});
  fs.mkdirSync(path.join(pack, 'nested/inline'), {recursive: true});
  fs.writeFileSync(path.join(pack, 'nested/inline/tree-sitter-inline.wasm'), 'inline');
  fs.writeFileSync(path.join(pack, 'tree-sitter.json'), JSON.stringify(manifest));
  assert.equal(stageLanguagePacks({source, destination: path.join(root, 'out')}).length, 2);
  assert.equal(fs.readFileSync(path.join(root, 'out/tree-sitter-rust/nested/inline/tree-sitter-inline.wasm'), 'utf8'), 'inline');
});

test('invalid standard paths never fall back to a legacy manifest', t => {
  const {root, source, pack, manifest} = standardFixture(t);
  fs.writeFileSync(path.join(pack, 'package.json'), '{}');
  for (const invalid of ['../escape.scm', '/absolute.scm', 'C:/absolute.scm', 'a\\bad.scm']) {
    manifest.grammars[0].highlights = invalid;
    fs.writeFileSync(path.join(pack, 'tree-sitter.json'), JSON.stringify(manifest));
    assert.throws(() => stageLanguagePacks({source, destination: path.join(root, 'invalid')}), /Invalid standard resource path/);
  }
});

test('standard packaging requires every declared highlight and a prebuilt module', t => {
  const {root, source, pack} = standardFixture(t);
  fs.unlinkSync(path.join(pack, 'queries/base.scm'));
  assert.throws(() => stageLanguagePacks({source, destination: path.join(root, 'missing-query')}), /ENOENT/);
  fs.writeFileSync(path.join(pack, 'queries/base.scm'), 'base');
  fs.unlinkSync(path.join(pack, 'tree-sitter-rust.wasm'));
  assert.throws(() => stageLanguagePacks({source, destination: path.join(root, 'missing-module')}), /Missing prebuilt language module/);
});
