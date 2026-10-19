import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import {execFileSync} from 'node:child_process';
import {extractLanguageArtifacts} from './fetch-languages.mjs';
import {moduleFile} from './lib/language-packs.mjs';

test('release assembly consumes standard modules unchanged and requires enabled defaults', t => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'loom-standard-release-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  const input = path.join(root, 'input');
  for (const id of ['json', 'markdown']) {
    const pack = path.join(input, id);
    fs.mkdirSync(path.join(pack, 'modules'), {recursive: true});
    fs.mkdirSync(path.join(pack, 'grammars', id), {recursive: true});
    fs.writeFileSync(path.join(pack, 'package.json'), JSON.stringify({schema: 1, id, languages: [], grammars: {[id]: {module: id}}}));
    fs.writeFileSync(path.join(pack, 'modules', moduleFile(id, process.platform)), 'original standard binary');
    for (const file of ['highlights.scm', 'SOURCE.md']) fs.writeFileSync(path.join(pack, 'grammars', id, file), 'resource');
  }
  const archive = path.join(root, 'languages.tar.gz');
  execFileSync('tar', ['-czf', archive, '-C', input, 'json', 'markdown']);
  const installed = path.join(root, 'installed');
  extractLanguageArtifacts(archive, installed);
  assert.equal(fs.readFileSync(path.join(installed, 'json/modules', moduleFile('json', process.platform)), 'utf8'), 'original standard binary');
  assert.throws(() => extractLanguageArtifacts(archive, installed), /must not exist/);
  const manifest = path.join(input, 'json/package.json');
  const value = JSON.parse(fs.readFileSync(manifest)); value.enabled = false; fs.writeFileSync(manifest, JSON.stringify(value));
  execFileSync('tar', ['-czf', archive, '-C', input, 'json', 'markdown']);
  assert.throws(() => extractLanguageArtifacts(archive, path.join(root, 'disabled')), /must be enabled/);
  assert.equal(fs.existsSync(path.join(root, 'disabled')), false);
});

test('release assembly accepts upstream-named standard defaults with no Loom manifests', t => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'loom-upstream-release-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  const input = path.join(root, 'input');
  for (const name of ['json', 'markdown']) {
    const pack = path.join(input, 'tree-sitter-' + name);
    fs.mkdirSync(path.join(pack, 'queries'), {recursive: true});
    fs.writeFileSync(path.join(pack, 'tree-sitter.json'), JSON.stringify({grammars: [{name}]}));
    fs.writeFileSync(path.join(pack, 'queries/highlights.scm'), 'query');
    fs.writeFileSync(path.join(pack, 'tree-sitter-' + name + '.wasm'), 'original');
  }
  const archive = path.join(root, 'upstream.tar.gz');
  execFileSync('tar', ['-czf', archive, '-C', input, 'tree-sitter-json', 'tree-sitter-markdown']);
  const destination = path.join(root, 'output');
  extractLanguageArtifacts(archive, destination);
  assert.equal(fs.readFileSync(path.join(destination, 'tree-sitter-json/tree-sitter-json.wasm'), 'utf8'), 'original');
  assert.equal(fs.existsSync(path.join(destination, 'tree-sitter-json/package.json')), false);
});
