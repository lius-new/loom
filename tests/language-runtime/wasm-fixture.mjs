// Verified, unmodified upstream WASM input. This prepares test data only and
// never invokes a grammar generator, C compiler or a language Cargo package.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {download, DOWNLOADS_DIR, sha256File} from '../../scripts/package/lib/util.mjs';

export const RUST_WASM = {
  url: 'https://github.com/tree-sitter/tree-sitter-rust/releases/download/v0.24.2/tree-sitter-rust.wasm',
  sha256: '24c89bd9252255e4aebbcbd7d2d308bd92c86dd95a130fdc80efa49577b8d738',
  file: 'tree-sitter-rust-0.24.2.wasm',
};
export async function prepareWasmFixture(destination, {module, offline = false} = {}) {
  if (!module) {
    module = path.join(DOWNLOADS_DIR, RUST_WASM.file);
    if (!offline) module = await download(RUST_WASM);
    if (!fs.existsSync(module)) throw new Error('Supply --wasm-rust with the upstream WASM file, or run once online to cache the pinned test input');
  }
  if (sha256File(module) !== RUST_WASM.sha256) throw new Error('Rust WASM regression input does not match the pinned upstream SHA-256');
  const root = path.dirname(fileURLToPath(import.meta.url));
  const target = path.join(destination, 'rust');
  fs.rmSync(target, {recursive: true, force: true});
  fs.mkdirSync(path.join(target, 'queries'), {recursive: true});
  for (const file of ['tree-sitter.json', 'queries/highlights.scm', 'queries/injections.scm']) {
    fs.copyFileSync(path.join(root, 'fixtures/rust', file), path.join(target, file));
  }
  // Preserve the upstream names and directory layout without editor metadata.
  fs.copyFileSync(module, path.join(target, 'tree-sitter-rust.wasm'));
  return path.resolve(destination);
}
