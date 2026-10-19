import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
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
