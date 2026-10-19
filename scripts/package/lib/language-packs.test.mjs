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
  const pack = path.join(profile, 'language-packs', 'rust');
  const query = path.join(pack, 'grammars', 'rust');
  fs.mkdirSync(query, { recursive: true });
  fs.writeFileSync(path.join(query, 'highlights.scm'), 'query');
  fs.writeFileSync(path.join(query, 'SOURCE.md'), 'license');
  fs.writeFileSync(path.join(pack, 'package.json'), JSON.stringify({
    schema: 1, id: 'rust', languages: [], servers: [],
    grammars: { rust: { module: 'loom_grammar_rust', symbol: 'loom_tree_sitter_rust' } },
  }));
  fs.writeFileSync(path.join(profile, moduleFile('loom_grammar_rust', platform)), 'native');
  return { root, profile, pack };
}

test('all platforms stage external modules and only manifest-listed queries', t => {
  for (const platform of ['win32', 'linux', 'darwin']) {
    const { root, profile, pack } = fixture(t, platform);
    fs.mkdirSync(path.join(pack, 'grammars', 'unused'));
    fs.writeFileSync(path.join(pack, 'grammars', 'unused', 'highlights.scm'), 'stale');
    const destination = path.join(root, 'installed');
    const binaries = stageLanguagePacks({ profiles: [profile], destination, platform });
    assert.equal(binaries.length, 1);
    assert.equal(fs.readFileSync(binaries[0], 'utf8'), 'native');
    assert.equal(fs.readFileSync(path.join(destination, 'rust/grammars/rust/highlights.scm'), 'utf8'), 'query');
    assert.equal(fs.existsSync(path.join(destination, 'rust/grammars/unused')), false);
    assert.equal(fs.existsSync(path.join(destination, 'rust/grammars/rust/injections.scm')), false);
  }
});

test('missing modules and malformed paths stop packaging', t => {
  const { root, profile, pack } = fixture(t, 'win32');
  fs.unlinkSync(path.join(profile, moduleFile('loom_grammar_rust', 'win32')));
  assert.throws(() => stageLanguagePacks({ profiles: [profile], destination: path.join(root, 'installed'), platform: 'win32' }), /Missing language module/);
  assert.throws(() => moduleFile('../escaped', 'win32'), /Invalid/);
  const catalog = JSON.parse(fs.readFileSync(path.join(pack, 'package.json')));
  catalog.grammars['../escaped'] = catalog.grammars.rust;
  fs.writeFileSync(path.join(pack, 'package.json'), JSON.stringify(catalog));
  assert.throws(() => stageLanguagePacks({ profiles: [profile], destination: path.join(root, 'invalid'), platform: 'win32' }), /Invalid grammar/);
});

test('universal packages combine each module once from both architectures', t => {
  const first = fixture(t, 'darwin');
  const second = fixture(t, 'darwin');
  const calls = [];
  const binaries = stageLanguagePacks({
    profiles: [first.profile, second.profile],
    destination: path.join(first.root, 'universal'),
    platform: 'darwin',
    combine(inputs, output) {
      calls.push(inputs);
      fs.writeFileSync(output, inputs.map(input => fs.readFileSync(input, 'utf8')).join('+'));
    },
  });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].length, 2);
  assert.equal(fs.readFileSync(binaries[0], 'utf8'), 'native+native');
  fs.writeFileSync(path.join(second.pack, 'package.json'), '{}');
  assert.throws(() => stageLanguagePacks({
    profiles: [first.profile, second.profile], destination: path.join(first.root, 'bad'), platform: 'darwin',
  }), /manifests differ/);
});
