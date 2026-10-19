import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {runtimeGrammarManifestErrors} from './check.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
test('repository ships language resources without a language producer or Rust wrappers', () => {
  for (const directory of ['language-packs', 'crates/language-packs']) assert.equal(fs.existsSync(path.join(root, directory)), false, directory);
  for (const file of ['Cargo.toml', 'crates/editor-core/Cargo.toml', 'crates/workspace-tests/Cargo.toml', 'tests/language-runtime/Cargo.toml']) {
    assert.deepEqual(runtimeGrammarManifestErrors(fs.readFileSync(path.join(root, file), 'utf8')), [], file);
  }
  const resources = path.join(root, 'assets/language-packs');
  for (const id of fs.readdirSync(resources).filter(id => fs.statSync(path.join(resources, id)).isDirectory())) {
    const manifest = JSON.parse(fs.readFileSync(path.join(resources, id, 'package.json')));
    assert.equal(manifest.id, id);
    assert.notEqual(manifest.enabled, false);
    for (const [grammar, module] of Object.entries(manifest.grammars)) {
      assert.equal(module.symbol, undefined, 'bundled resources use standard inferred exports');
      assert.equal(module.module, grammar);
      for (const file of ['highlights.scm', 'SOURCE.md']) assert.ok(fs.statSync(path.join(resources, id, 'grammars', grammar, file)).size > 0);
    }
  }
  for (const id of ['json', 'markdown']) assert.ok(fs.existsSync(path.join(resources, id, 'package.json')));
  for (const manifest of ['Cargo.toml', 'tests/language-runtime/Cargo.toml']) {
    assert.match(fs.readFileSync(path.join(root, manifest), 'utf8'), /tree-sitter\s*=\s*\{[^}]*features\s*=\s*\[[^\]]*"wasm"/);
  }
  for (const workflow of ['ci.yml', 'release-linux.yml', 'release-macos.yml', 'release-windows.yml']) {
    const text = fs.readFileSync(path.join(root, '.github/workflows', workflow), 'utf8');
    assert.doesNotMatch(text, /language-packs\/build|loom-grammar-/);
    assert.match(text, /fetch-languages\.mjs/);
    assert.match(text, /run-wasm\.mjs/, 'WASM regressions run even without a native resource feed');
  }
  assert.match(fs.readFileSync(path.join(root, 'scripts/package/macos/entitlements.plist'), 'utf8'), /com\.apple\.security\.cs\.allow-jit/);
});
