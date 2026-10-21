import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { stageLspServers } from './lsp-servers.mjs';

const temporary = () => fs.mkdtempSync(path.join(os.tmpdir(), 'loom-lsp-stage-'));

test('the published catalog is staged unchanged as the only resource', () => {
  const root = temporary();
  const staged = stageLspServers({ destination: path.join(root, 'lsp-servers') });
  assert.equal(path.basename(staged), 'lsp-servers.toml');
  assert.deepEqual(fs.readdirSync(path.join(root, 'lsp-servers')), ['lsp-servers.toml']);
  const text = fs.readFileSync(staged, 'utf8');
  assert.match(text, /^schema = 1$/m);
  assert.equal(text, fs.readFileSync(new URL('../../../assets/lsp-servers/lsp-servers.toml', import.meta.url), 'utf8'));
});

test('a missing catalog or destination is an error', () => {
  const root = temporary();
  assert.throws(() => stageLspServers({ catalog: path.join(root, 'missing.toml'), destination: path.join(root, 'out') }), /Missing language server catalog/);
  assert.throws(() => stageLspServers({}), /destination/);
});
