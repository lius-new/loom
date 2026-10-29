import fs from 'node:fs';
import path from 'node:path';
import {downloadVerified, sha256} from '../lib/download.mjs';
export {sha256} from '../lib/download.mjs';
import {execFileSync} from 'node:child_process';

// Only complete, verified downloads are published. Concurrent preparers use
// unique temporary paths and can safely publish the same pinned content.
export async function archive(spec, {cache, offline = false, ...options} = {}) {
  const directory = path.join(cache, 'archives', spec.sha256);
  const file = path.join(directory, spec.file);
  if (fs.existsSync(file) && await sha256(file) === spec.sha256) return file;
  if (offline) throw Error(`Missing or corrupt dependency cache: ${file}. Run node scripts/dependencies/ensure.mjs online first.`);
  console.error('Preparing ' + spec.file);
  return downloadVerified({url:spec.url, destination:file, sha256:spec.sha256}, options);
}

export function tar(args) {
  const executable = process.platform === 'win32'
    ? path.join(process.env.SystemRoot || 'C:/Windows', 'System32', 'tar.exe') : 'tar';
  return execFileSync(executable, args, {encoding: 'utf8', maxBuffer: 16 * 1024 * 1024});
}

async function inventory(directory, relative = '') {
  const records = {};
  for (const entry of fs.readdirSync(path.join(directory, relative), {withFileTypes: true})) {
    const name = path.posix.join(relative, entry.name), file = path.join(directory, name);
    if (entry.isDirectory()) Object.assign(records, await inventory(directory, name));
    else if (entry.isSymbolicLink()) {
      const link = fs.readlinkSync(file), resolved = path.resolve(path.dirname(file), link);
      if (!resolved.startsWith(path.resolve(directory) + path.sep)) throw Error(`Escaping dependency symlink: ${name}`);
      records[name] = {link};
    } else records[name] = {sha256: await sha256(file)};
  }
  return records;
}

async function locked(directory, callback) {
  fs.mkdirSync(directory, {recursive: true});
  const lock = path.join(directory, 'prepare.lock');
  const deadline = Date.now() + 360_000;
  for (;;) {
    let handle;
    try { handle = fs.openSync(lock, 'wx'); }
    catch (error) {
      if (error.code !== 'EEXIST') throw error;
      try {
        const owner = Number(fs.readFileSync(lock, 'utf8'));
        if (owner > 0) {try {process.kill(owner, 0);} catch (error) {if (error.code === 'ESRCH') fs.rmSync(lock, {force: true});}}
      } catch { /* Another preparer may just have removed the lock. */ }
      if (Date.now() >= deadline) throw Error(`Timed out waiting for dependency preparation: ${lock}`);
      await new Promise(resolve => setTimeout(resolve, 100));
      continue;
    }
    fs.writeFileSync(handle, String(process.pid));
    try {return await callback();}
    finally {fs.closeSync(handle); fs.rmSync(lock, {force: true});}
  }
}

export async function extract(spec, file, {cache, extractor = tar} = {}) {
  const parent = path.join(cache, 'tools', spec.sha256), destination = path.join(parent, 'content');
  return locked(parent, async () => {
    const receipt = path.join(parent, 'receipt.json');
    try {
      const expected = JSON.parse(fs.readFileSync(receipt, 'utf8'));
      if (Object.keys(expected).length && JSON.stringify(await inventory(destination)) === JSON.stringify(expected)) return destination;
    } catch { /* Missing or damaged extraction is rebuilt from the verified archive. */ }
    fs.mkdirSync(parent, {recursive: true});
    const temporary = fs.mkdtempSync(path.join(parent, 'extract-'));
    try {
      const entries = extractor(['-tf', file]).split(/\r?\n/).filter(Boolean);
      if (entries.some(name => /^(?:[/\\]|[A-Za-z]:)/.test(name) || name.split(/[/\\]/).includes('..'))) throw Error('Unsafe dependency archive path');
      extractor(['-xf', file, '-C', temporary]);
      const records = await inventory(temporary);
      if (!Object.keys(records).length) throw Error('Empty dependency archive');
      // All paths removed here are fixed children of our SHA-addressed cache.
      fs.rmSync(destination, {recursive: true, force: true});
      fs.renameSync(temporary, destination);
      fs.writeFileSync(receipt, JSON.stringify(records));
      return destination;
    } finally { fs.rmSync(temporary, {recursive: true, force: true}); }
  });
}
