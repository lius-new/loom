// Starts every built server of this machine's platform and completes an LSP
// initialize / shutdown exchange with it. This runs in the workflow on each runner
// platform; it is not part of an ordinary build.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {pathToFileURL, fileURLToPath} from 'node:url';
import {WORK, RECIPES, assert, loadRecipes, hostPlatform, assetName, options, main, tar} from './lib.mjs';

/** Replaces ${pack} and ${runtime.<id>} in text. */
export function expand(text, {pack, runtimes}) {
  return text.replaceAll('${pack}', pack).replace(/\$\{runtime\.([a-z0-9_-]+)\}/g, (_, id) => {
    assert(runtimes[id], `No installed runtime: ${id}`);
    return runtimes[id];
  });
}
export const expandValue = (value, context) => typeof value === 'string' ? expand(value, context)
  : Array.isArray(value) ? value.map(item => expandValue(item, context))
    : value && typeof value === 'object' ? Object.fromEntries(Object.entries(value).map(([k, v]) => [k, expandValue(v, context)])) : value;

/** A command names an executable without its extension; on Windows the file has .exe. */
export function resolveCommand(command, platform = process.platform) {
  if (platform === 'win32' && !fs.existsSync(command) && fs.existsSync(command + '.exe')) return command + '.exe';
  return command;
}

const frame = message => { const body = Buffer.from(JSON.stringify({jsonrpc: '2.0', ...message})); return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]); };

/** Runs an LSP handshake against a server and returns its capabilities. */
export function handshake({command, args = [], cwd, initializationOptions, timeout = 60000}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {cwd, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true});
    let buffer = Buffer.alloc(0), stderr = '', settled = false, capabilities = null;
    // Settles only once the process is gone, so its working directory can be removed.
    const finish = (error, value, {spawned = true} = {}) => {
      if (settled) return; settled = true; clearTimeout(timer);
      const done = () => {
        if (error) reject(Object.assign(error, {message: `${error.message}${stderr ? `\nstderr: ${stderr.slice(-800)}` : ''}`}));
        else resolve(value);
      };
      if (spawned && child.exitCode === null && child.signalCode === null) { child.once('exit', done); child.kill(); } else done();
    };
    const timer = setTimeout(() => finish(Error(`Timed out after ${timeout} ms`)), timeout);
    child.on('error', error => finish(error, undefined, {spawned: false}));
    child.on('exit', code => { if (!settled && !capabilities) finish(Error(`The server exited (${code}) before initializing`)); });
    child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-4000); });
    const send = message => child.stdin.write(frame(message));
    const handle = message => {
      if (message.method && message.id !== undefined) {
        // Server requests are answered with an empty result so it never waits on us.
        send({id: message.id, result: message.method === 'workspace/configuration' ? (message.params?.items || []).map(() => null) : null});
      } else if (message.id === 1) {
        if (message.error) return finish(Error(`initialize failed: ${JSON.stringify(message.error)}`));
        capabilities = message.result?.capabilities;
        if (!capabilities) return finish(Error('initialize returned no capabilities'));
        send({method: 'initialized', params: {}});
        send({id: 2, method: 'shutdown'});
      } else if (message.id === 2) {
        send({method: 'exit'});
        child.stdin.end();
        const wait = setTimeout(() => child.kill(), 5000);
        child.on('exit', () => { clearTimeout(wait); finish(null, capabilities); });
      }
    };
    child.stdout.on('data', chunk => {
      buffer = Buffer.concat([buffer, chunk]);
      for (;;) {
        const end = buffer.indexOf('\r\n\r\n');
        if (end < 0) return;
        const length = Number(/content-length:\s*(\d+)/i.exec(buffer.toString('ascii', 0, end))?.[1]);
        if (!Number.isFinite(length)) return finish(Error('The server sent a malformed header'));
        if (buffer.length < end + 4 + length) return;
        const body = buffer.subarray(end + 4, end + 4 + length); buffer = buffer.subarray(end + 4 + length);
        try { handle(JSON.parse(body.toString('utf8'))); } catch (error) { return finish(error); }
      }
    });
    send({id: 1, method: 'initialize', params: {processId: process.pid, clientInfo: {name: 'loom-lsp-validate'},
      rootUri: pathToFileURL(cwd).href, workspaceFolders: null, capabilities: {}, ...(initializationOptions ? {initializationOptions} : {})}});
  });
}

async function unpack(dist, id, platform, destination) {
  const archive = path.join(dist, assetName(id, platform));
  assert(fs.existsSync(archive), `Missing build artifact: ${archive}`);
  fs.mkdirSync(destination, {recursive: true});
  await tar.x({file: archive, cwd: destination, strict: true});
  return path.join(destination, id);
}

/** A GitHub Actions annotation, readable without opening the log. */
export const annotation = (title, message) => {
  const escape = value => String(value).replace(/%/g, '%25').replace(/\r/g, '%0D').replace(/\n/g, '%0A');
  return `::error title=${escape(title).replace(/:/g, '%3A').replace(/,/g, '%2C')}::${escape(message)}`;
};

/**
 * Starts every server of this platform. A failing server does not stop the others:
 * all failures are collected and reported together, once the rest have run.
 */
export async function validateServers(args = {}) {
  const recipes = loadRecipes(args.recipes || RECIPES), work = args.work || WORK, dist = path.join(work, 'dist');
  const report = JSON.parse(fs.readFileSync(path.join(dist, 'build-report.json'), 'utf8'));
  const host = hostPlatform(), temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'loom-lsp-validate-'));
  const results = [], failures = [];
  const fail = (name, error) => { failures.push({name, error: error.message}); console.log(annotation(`${name} failed on ${host}`, error.message)); };
  try {
    for (const id of report.selected) {
      const pack = recipes.packs[id], platforms = Object.keys(report.packs[id].platforms);
      const platform = platforms.includes('any') ? 'any' : host;
      if (!platforms.includes(platform)) { console.log(`skip ${id}: not built for ${host}`); continue; }
      const servers = Object.entries(recipes.servers).filter(([, server]) => server.pack === id);
      if (!servers.length) { console.log(`skip ${id}: a runtime with no server of its own`); continue; }
      let directory;
      const runtimes = {};
      try {
        directory = await unpack(dist, id, platform, path.join(temporary, id));
        for (const requirement of pack.requires || []) {
          const runtime = requirement.split('@')[0];
          assert(report.packs[runtime]?.platforms[host], `${id} needs ${runtime}, which was not built for ${host}`);
          runtimes[runtime] ||= await unpack(dist, runtime, host, path.join(temporary, runtime));
        }
      } catch (error) { fail(id, error); continue; }
      for (const [name, server] of servers) {
        try {
          const context = {pack: directory, runtimes}, root = path.join(temporary, `${name}-project`);
          fs.mkdirSync(root, {recursive: true});
          const command = resolveCommand(expand(server.command, context));
          const capabilities = await handshake({command, args: (server.args || []).map(arg => expand(arg, context)), cwd: root,
            initializationOptions: server.initialization_options ? expandValue(server.initialization_options, context) : undefined});
          console.log(`ok ${name} (${host}): ${Object.keys(capabilities).length} capabilities`);
          results.push(name);
        } catch (error) { fail(name, error); }
      }
    }
  } finally { fs.rmSync(temporary, {recursive: true, force: true, maxRetries: 20, retryDelay: 200}); }
  assert(failures.length === 0, `${failures.length} server(s) failed on ${host}: ${failures.map(f => f.name).join(', ')} (${results.length} passed)`);
  assert(results.length > 0, 'Nothing was validated');
  return results;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main(async () => {
  const args = options(process.argv.slice(2));
  if (args.help) { console.log('node scripts/lsp-servers/validate.mjs [--work directory]'); return; }
  await validateServers(args);
});
