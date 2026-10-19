// A complete WASM-only regression, with no native language artifact collection.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {prepareWasmFixture} from './wasm-fixture.mjs';

const root = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const option = name => { const index = args.indexOf(name); return index < 0 ? null : args[index + 1]; };
const packages = await prepareWasmFixture(path.join(root, 'target/wasm-input'), {module: option('--wasm-rust'), offline: args.includes('--offline')});
const env = {...process.env, LOOM_TEST_WASM_PACKAGES: packages};
delete env.LOOM_TEST_LANGUAGE_PACKS;
const cargo = command => execFileSync('cargo', [...command, '--manifest-path', path.join(root, 'Cargo.toml'), '--locked', ...(args.includes('--offline') ? ['--offline'] : []), '--target-dir', path.join(root, 'target')], {cwd: root, env, stdio: 'inherit'});
if (!args.includes('--probe-only')) cargo(['test', '--lib', 'infrastructure::language_packs::wasm_tests::']);
cargo(['build', '--bin', 'language-runtime-probe']);
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'loom-wasm-installed-'));
try {
  const binary = path.join(temp, process.platform === 'win32' ? 'probe.exe' : 'probe');
  fs.copyFileSync(path.join(root, 'target/debug', process.platform === 'win32' ? 'language-runtime-probe.exe' : 'language-runtime-probe'), binary);
  if (process.platform !== 'win32') fs.chmodSync(binary, 0o755);
  const digest = () => createHash('sha256').update(fs.readFileSync(binary)).digest('hex');
  const original = digest();
  const home = path.join(temp, 'user');
  const probeEnv = {...process.env, LOCALAPPDATA: home, HOME: home, XDG_DATA_HOME: home};
  const user = path.join(home, ...(process.platform === 'win32' ? ['Loom'] : process.platform === 'darwin' ? ['Library', 'Application Support', 'Loom'] : ['loom']), 'language-packs');
  fs.writeFileSync(path.join(temp, 'sample.rs'), 'fn hello() { let value = r#"raw"#; /* /* nested */ */ }\n');
  fs.writeFileSync(path.join(temp, 'sample.downloaded'), 'fn downloaded() {}\n');
  const probe = file => JSON.parse(execFileSync(binary, [path.join(temp, file)], {cwd: temp, env: probeEnv, encoding: 'utf8'}));
  const plain = {language: 'plaintext', colored: 0, modules: [], wasm_modules: [], errors: []};
  assert.deepEqual(probe('sample.rs'), plain);
  const portable = path.join(temp, 'language-packs');
  fs.cpSync(path.join(packages, 'rust'), path.join(portable, 'rust'), {recursive: true});
  const installed = probe('sample.rs');
  assert.equal(installed.language, 'rust'); assert.ok(installed.colored > 0);
  assert.deepEqual(installed.wasm_modules, ['tree-sitter-rust.wasm']); assert.deepEqual(installed.errors, []);
  assert.deepEqual(fs.readdirSync(path.join(portable, 'rust')).sort(), ['queries', 'tree-sitter-rust.wasm', 'tree-sitter.json'].sort());
  fs.mkdirSync(path.dirname(user), {recursive: true}); fs.renameSync(portable, user);
  assert.deepEqual(probe('sample.rs'), installed);
  assert.deepEqual(probe('sample.downloaded'), plain);
  const manifest = path.join(user, 'rust/tree-sitter.json');
  const upstream = JSON.parse(fs.readFileSync(manifest));
  upstream.grammars[0]['file-types'].push('downloaded');
  fs.writeFileSync(manifest, JSON.stringify(upstream));
  const extra = probe('sample.downloaded');
  assert.equal(extra.language, 'rust'); assert.ok(extra.colored > 0); assert.deepEqual(extra.errors, []);
  assert.deepEqual(extra.wasm_modules, ['tree-sitter-rust.wasm']);
  fs.renameSync(path.join(user, 'rust'), path.join(home, 'removed-rust'));
  assert.deepEqual(probe('sample.rs'), plain);
  assert.equal(digest(), original);
  console.log(JSON.stringify({wasmOnly: true, installed, extra, portableAndUser: true, unchangedExecutable: true, removedOnRestart: true}));
} finally {
  assert.equal(path.dirname(temp), os.tmpdir());
  fs.rmSync(temp, {recursive: true, force: true});
}
