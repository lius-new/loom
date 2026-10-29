import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';

function canonicalPath(value) {
  const resolved = path.resolve(value);
  try { return fs.realpathSync(resolved); }
  catch (error) {
    // Missing custom paths must still be rejected as configuration conflicts.
    if (error.code !== 'ENOENT' && error.code !== 'ENOTDIR') throw error;
    return resolved;
  }
}

export function install(cwd = process.cwd()) {
  const git = args => execFileSync('git', args, {cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe']}).trim();
  const root = git(['rev-parse', '--show-toplevel']);
  const hooks = path.join(root, '.githooks');
  const template = path.join(root, '.github', 'commit-message.txt');
  const expected = [['core.hooksPath', hooks], ['commit.template', template]];
  // Inspect both settings before writing either; preserve existing custom tooling.
  for (const [key, value] of expected) {
    let current;
    try { current = git(['config', '--path', '--get', key]); }
    catch (error) { if (error.status !== 1) throw error; }
    if (current && canonicalPath(path.resolve(root, current)) !== canonicalPath(value)) throw new Error('Existing ' + key + ' is configured: ' + current + '. Integrate the Release Notes checker with that tooling before changing it.');
  }
  const defaultHooks = path.resolve(root, git(['rev-parse', '--git-path', 'hooks']));
  const existingHooks = fs.existsSync(defaultHooks) ? fs.readdirSync(defaultHooks).filter(name => !name.endsWith('.sample') && !name.startsWith('.') && fs.statSync(path.join(defaultHooks, name)).isFile()) : [];
  if (existingHooks.length) {
    let configured;
    try { configured = git(['config', '--get', 'core.hooksPath']); } catch (error) { if (error.status !== 1) throw error; }
    if (!configured) throw new Error('Existing .git/hooks/' + existingHooks.join(', ') + ' found; integrate the Release Notes checker with existing hooks first.');
  }
  fs.chmodSync(path.join(hooks, 'commit-msg'), 0o755);
  for (const [key, value] of expected) git(['config', '--local', key, value]);
  return root;
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { console.log('Release Notes hook and commit template enabled for ' + install()); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
