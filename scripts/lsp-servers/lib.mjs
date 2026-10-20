// Language server pack production. Shares the pinned-download, archive and TOML
// helpers of the language package pipeline; the recipes, assembly and catalog here
// are specific to language servers.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import zlib from 'node:zlib';
import {createHash} from 'node:crypto';
import {ROOT, assert, relative, inside, hash, fileHash, canonical, files, writeAtomic, removeWork, run,
  createArchive, fetchPinned, HASH, parseToml, stringifyToml, tar} from '../language-packs/lib.mjs';

export {ROOT, assert, hash, fileHash, canonical, files, writeAtomic, run, createArchive, fetchPinned, HASH, tar};

export const HERE = path.resolve(fileURLToPath(new URL('.', import.meta.url)));
export const RECIPES = path.join(HERE, 'build.toml');
export const PINS = path.join(HERE, 'pins.toml');
export const NPM = path.join(HERE, 'npm');
export const CATALOG = path.join(ROOT, 'assets/lsp-servers/lsp-servers.toml');
export const WORK = path.join(ROOT, '.packaging/lsp-servers');

export const PLATFORMS = ['windows-x86_64', 'windows-aarch64', 'macos-x86_64', 'macos-aarch64', 'linux-x86_64', 'linux-aarch64'];
/** Platform of the machine running the pipeline, in the catalog's vocabulary. */
export function hostPlatform() {
  const os = {win32: 'windows', darwin: 'macos', linux: 'linux'}[process.platform];
  const arch = {x64: 'x86_64', arm64: 'aarch64'}[process.arch];
  assert(os && arch, `Unsupported host: ${process.platform}-${process.arch}`);
  return `${os}-${arch}`;
}
export const FORMATS = ['zip', 'tar.gz', 'tar.xz', 'gz', 'raw'];
export const MAX_ARCHIVE = 256 * 1024 * 1024;
export const RELEASE = /^lsp-servers-[a-zA-Z0-9][a-zA-Z0-9._-]*$/;
const PACK_ID = /^[a-z][a-z0-9_-]*$/;
const LANGUAGE = /^[a-z][a-z0-9_]*$/;
const FILE_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;
const EXACT_PACKAGE = /^(@[\w.-]+\/)?[\w.-]+@\d+\.\d+\.\d+$/;

export const readToml = file => parseToml(fs.readFileSync(file, 'utf8'));
export const stableToml = (header, data) => header + stringifyToml(data);

/** The platforms a pack is built and published for. */
export const packPlatforms = pack => pack.kind === 'npm' ? ['any'] : Object.keys(pack.platforms);
export const assetName = (id, platform) => `${id}-${platform}.tar.gz`;
const major = version => String(version).split('.')[0];
const placeholder = /^\$\{(pack|runtime\.[a-z][a-z0-9_-]*)\}\//;

function checkServer(id, server, packs) {
  assert(PACK_ID.test(id), `Invalid server id: ${id}`);
  assert(typeof server.name === 'string' && server.name.trim() && server.name === server.name.trim(), `Missing server name: ${id}`);
  const pack = packs[server.pack];
  assert(pack, `Server ${id} names an unknown pack: ${server.pack}`);
  const match = placeholder.exec(server.command || '');
  assert(match, `Server ${id}: the command must start with \${pack}/ or \${runtime.<id>}/ and never names a program found on PATH`);
  if (match[1] !== 'pack') {
    const runtime = match[1].slice('runtime.'.length);
    assert((pack.requires || []).some(req => req.split('@')[0] === runtime), `Server ${id} uses a runtime its pack does not require: ${runtime}`);
  }
  assert(!/[\0-\x1f]/.test(server.command), `Invalid command: ${id}`);
  for (const arg of server.args || []) assert(typeof arg === 'string' && !/[\0-\x1f]/.test(arg), `Invalid argument: ${id}`);
  assert(server.priority === undefined || Number.isSafeInteger(server.priority), `Invalid priority: ${id}`);
  assert(server.initialization_options === undefined || (server.initialization_options && typeof server.initialization_options === 'object' && !Array.isArray(server.initialization_options)), `initialization_options must be an object: ${id}`);
  const root = server.root;
  assert(root && Array.isArray(root.markers) && root.markers.length && root.markers.every(m => typeof m === 'string' && m.trim() && !/[\0-\x1f/\\]/.test(m)), `Invalid root markers: ${id}`);
  assert(root.strategy === undefined || ['nearest', 'outermost'].includes(root.strategy), `Invalid root strategy: ${id}`);
  assert(['workspace', 'file_directory', 'none'].includes(root.fallback), `Invalid root fallback: ${id}`);
  assert(Array.isArray(server.languages) && server.languages.length, `A server serves at least one language: ${id}`);
  const seen = new Set();
  for (const binding of server.languages) {
    assert(LANGUAGE.test(binding.language) && (binding.variant === undefined || LANGUAGE.test(binding.variant)), `Invalid language binding: ${id}`);
    assert(typeof binding.language_id === 'string' && binding.language_id && !/[\s\0-\x1f]/.test(binding.language_id), `Invalid language_id: ${id}`);
    const key = `${binding.language}/${binding.variant ?? ''}`;
    assert(!seen.has(key), `Language bound twice: ${id} ${key}`); seen.add(key);
  }
}

export function validateRecipes(data) {
  assert(data.schema === 1 && data.packs && data.servers, 'Expected recipe schema 1, packs and servers');
  assert(Object.keys(data).every(key => ['schema', 'packs', 'servers'].includes(key)), 'Unexpected recipe section');
  for (const [id, pack] of Object.entries(data.packs)) {
    assert(PACK_ID.test(id), `Invalid pack id: ${id}`);
    assert(['archive', 'npm', 'go'].includes(pack.kind), `Unknown pack kind: ${id}`);
    assert(typeof pack.version === 'string' && /^[\w.+-]+$/.test(pack.version), `Pin a version: ${id}`);
    assert(typeof pack.license === 'string' && pack.license.trim(), `Missing license: ${id}`);
    for (const requirement of pack.requires || []) {
      const [runtime, wanted] = String(requirement).split('@');
      assert(PACK_ID.test(runtime) && /^\d+$/.test(wanted ?? ''), `Invalid requirement in ${id}: ${requirement}`);
      assert(data.packs[runtime] && major(data.packs[runtime].version) === wanted, `${id} requires ${requirement}, which no pack provides`);
    }
    const licenseNames = new Set();
    for (const license of pack.licenses || []) {
      assert(pack.kind !== 'npm', `An npm pack carries its licenses in node_modules: ${id}`);
      assert(FILE_NAME.test(license.name) && !licenseNames.has(license.name) && new URL(license.url).protocol === 'https:', `Invalid license reference: ${id}`);
      licenseNames.add(license.name);
    }
    if (pack.kind === 'go') assert(pack.licenses?.length, `A Go pack names its license files: ${id}`);
    if (pack.kind === 'npm') {
      assert(pack.platforms === undefined, `An npm pack is built once for every platform: ${id}`);
      assert(Array.isArray(pack.packages) && pack.packages.length && pack.packages.every(p => EXACT_PACKAGE.test(p)), `Pin exact npm packages: ${id}`);
      continue;
    }
    assert(pack.platforms && Object.keys(pack.platforms).length, `A pack needs at least one platform: ${id}`);
    for (const [platform, spec] of Object.entries(pack.platforms)) {
      assert(PLATFORMS.includes(platform), `Unknown platform in ${id}: ${platform}`);
      assert(Array.isArray(spec.executables) && spec.executables.length, `Name the executables: ${id} ${platform}`);
      spec.executables.forEach(file => relative(file, 'executable'));
    }
    if (pack.kind === 'go') {
      assert(/^\d+\.\d+\.\d+$/.test(pack.go) && /^v\d+\.\d+\.\d+$/.test(pack['module-version']) && /^[\w./-]+$/.test(pack.module), `Pin the Go toolchain and module: ${id}`);
      for (const [platform, spec] of Object.entries(pack.platforms)) assert(/^[a-z0-9]+$/.test(spec.goos) && /^[a-z0-9]+$/.test(spec.goarch), `Invalid Go target: ${id} ${platform}`);
      continue;
    }
    const upstream = pack.upstream || {};
    assert(['nodejs', 'github'].includes(upstream.source), `Unknown upstream source: ${id}`);
    if (upstream.source === 'github') assert(/^[\w.-]+\/[\w.-]+$/.test(upstream.repository) && upstream.tag, `Pin a GitHub repository and tag: ${id}`);
    for (const [platform, spec] of Object.entries(pack.platforms)) {
      assert(typeof spec.asset === 'string' && !/[\\/\0]/.test(spec.asset), `Invalid asset: ${id} ${platform}`);
      assert(FORMATS.includes(spec.format), `Unknown format: ${id} ${platform}`);
      if (['gz', 'raw'].includes(spec.format)) assert(spec.executables.length === 1 && !spec.include, `A single-file asset names one executable: ${id} ${platform}`);
      for (const item of spec.include || []) {
        const from = typeof item === 'string' ? item : item.from, to = typeof item === 'string' ? item : item.to;
        relative(from, 'include'); relative(to, 'include');
      }
    }
  }
  for (const [id, server] of Object.entries(data.servers)) checkServer(id, server, data.packs);
  return data;
}
export const loadRecipes = (file = RECIPES) => validateRecipes(readToml(file));

export function assetUrl(pack, platform) {
  const spec = pack.platforms[platform], upstream = pack.upstream;
  if (upstream.source === 'nodejs') return `https://nodejs.org/dist/v${pack.version}/${spec.asset}`;
  return `https://github.com/${upstream.repository}/releases/download/${encodeURIComponent(upstream.tag)}/${encodeURIComponent(spec.asset)}`;
}

/** The pins every archive and license download needs; nothing is trusted without one. */
export function validatePins(recipes, pins) {
  assert(pins.schema === 1 && pins.packs, 'Expected pins schema 1');
  const expected = new Set();
  for (const [id, pack] of Object.entries(recipes.packs)) {
    for (const platform of pack.kind === 'archive' ? Object.keys(pack.platforms) : []) {
      const pin = pins.packs[id]?.platforms?.[platform];
      assert(pin, `Missing pin: ${id} ${platform}; run pin.mjs`);
      assert(pin.url === assetUrl(pack, platform), `Stale pin: ${id} ${platform}; run pin.mjs`);
      assert(HASH.test(pin.sha256) && Number.isSafeInteger(pin.size) && pin.size > 0, `Invalid pin: ${id} ${platform}`);
      expected.add(`${id}/${platform}`);
    }
    for (const license of pack.licenses || []) {
      const pin = pins.packs[id]?.licenses?.[license.name];
      assert(pin && pin.url === license.url && HASH.test(pin.sha256), `Missing or stale license pin: ${id} ${license.name}; run pin.mjs`);
      expected.add(`${id}/license/${license.name}`);
    }
  }
  for (const [id, pack] of Object.entries(pins.packs)) {
    for (const platform of Object.keys(pack.platforms || {})) assert(expected.has(`${id}/${platform}`), `Pin for no recipe: ${id} ${platform}`);
    for (const name of Object.keys(pack.licenses || {})) assert(expected.has(`${id}/license/${name}`), `Pin for no recipe: ${id} ${name}`);
  }
  return pins;
}
export const loadPins = (file = PINS) => fs.existsSync(file) ? readToml(file) : {schema: 1, packs: {}};

/** Lockfiles pin every dependency of an npm pack by integrity hash. */
export function npmDirectory(id, root = HERE) { return path.join(root, 'npm', id); }
export function validateLocks(recipes, root = HERE) {
  for (const [id, pack] of Object.entries(recipes.packs)) {
    if (pack.kind !== 'npm') continue;
    const directory = npmDirectory(id, root);
    const manifest = path.join(directory, 'package.json'), lock = path.join(directory, 'package-lock.json');
    assert(fs.existsSync(manifest) && fs.existsSync(lock), `Missing npm lockfile: ${id}; run pin.mjs`);
    const dependencies = JSON.parse(fs.readFileSync(manifest, 'utf8')).dependencies || {};
    const locked = JSON.parse(fs.readFileSync(lock, 'utf8'));
    assert(locked.lockfileVersion >= 2 && locked.packages, `Unsupported lockfile: ${id}`);
    for (const spec of pack.packages) {
      const at = spec.lastIndexOf('@'), name = spec.slice(0, at), version = spec.slice(at + 1);
      assert(dependencies[name] === version, `${id}: package.json does not pin ${spec}; run pin.mjs`);
      assert(locked.packages[`node_modules/${name}`]?.version === version, `${id}: the lockfile does not pin ${spec}; run pin.mjs`);
    }
    for (const [name, entry] of Object.entries(locked.packages)) {
      if (name === '' || entry.link) continue;
      assert(/^sha(256|384|512)-/.test(entry.integrity || ''), `${id}: ${name} has no integrity hash in the lockfile`);
    }
  }
}

export function inputDigest(recipes, pins, root = HERE) {
  const locks = {};
  for (const [id, pack] of Object.entries(recipes.packs)) {
    if (pack.kind !== 'npm') continue;
    const directory = npmDirectory(id, root);
    for (const name of ['package.json', 'package-lock.json']) locks[`${id}/${name}`] = fileHash(path.join(directory, name));
  }
  return hash(JSON.stringify(canonical({recipes, pins, locks})));
}

// ------------------------------------------------------------------ extraction

function crc32(buffer) {
  if (zlib.crc32) return zlib.crc32(buffer) >>> 0;
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
  }
  return (crc ^ 0xffffffff) >>> 0;
}

/** A small zip reader: stored and deflated entries, zip64 sizes, no links. */
export function extractZip(file, destination) {
  const buffer = fs.readFileSync(file);
  let end = buffer.length - 22;
  while (end >= 0 && buffer.readUInt32LE(end) !== 0x06054b50) end--;
  assert(end >= 0, `Not a zip archive: ${file}`);
  let count = buffer.readUInt16LE(end + 10), offset = buffer.readUInt32LE(end + 16);
  if ((count === 0xffff || offset === 0xffffffff) && end >= 20 && buffer.readUInt32LE(end - 20) === 0x07064b50) {
    const record = Number(buffer.readBigUInt64LE(end - 20 + 8));
    assert(buffer.readUInt32LE(record) === 0x06064b50, 'Unreadable zip64 directory');
    count = Number(buffer.readBigUInt64LE(record + 32)); offset = Number(buffer.readBigUInt64LE(record + 48));
  }
  fs.mkdirSync(destination, {recursive: true});
  let cursor = offset, total = 0;
  for (let index = 0; index < count; index++) {
    assert(buffer.readUInt32LE(cursor) === 0x02014b50, 'Corrupt zip directory');
    const flags = buffer.readUInt16LE(cursor + 8), method = buffer.readUInt16LE(cursor + 10);
    let compressed = buffer.readUInt32LE(cursor + 20), size = buffer.readUInt32LE(cursor + 24);
    const nameLength = buffer.readUInt16LE(cursor + 28), extraLength = buffer.readUInt16LE(cursor + 30), commentLength = buffer.readUInt16LE(cursor + 32);
    const attributes = buffer.readUInt32LE(cursor + 38);
    let local = buffer.readUInt32LE(cursor + 42);
    const name = buffer.toString('utf8', cursor + 46, cursor + 46 + nameLength);
    const extraStart = cursor + 46 + nameLength;
    if (size === 0xffffffff || compressed === 0xffffffff || local === 0xffffffff) {
      for (let at = extraStart; at + 4 <= extraStart + extraLength;) {
        const id = buffer.readUInt16LE(at), length = buffer.readUInt16LE(at + 2);
        if (id === 1) {
          let field = at + 4;
          if (size === 0xffffffff) { size = Number(buffer.readBigUInt64LE(field)); field += 8; }
          if (compressed === 0xffffffff) { compressed = Number(buffer.readBigUInt64LE(field)); field += 8; }
          if (local === 0xffffffff) local = Number(buffer.readBigUInt64LE(field));
        }
        at += 4 + length;
      }
    }
    cursor = extraStart + extraLength + commentLength;
    assert((flags & 1) === 0, `Encrypted zip entry: ${name}`);
    if (name.endsWith('/')) continue;
    const mode = attributes >>> 16;
    if ((mode & 0o170000) === 0o120000) continue; // links are never extracted
    relative(name, 'archive entry');
    assert(buffer.readUInt32LE(local) === 0x04034b50, `Corrupt zip entry: ${name}`);
    const start = local + 30 + buffer.readUInt16LE(local + 26) + buffer.readUInt16LE(local + 28);
    const data = buffer.subarray(start, start + compressed);
    const content = method === 0 ? data : method === 8 ? zlib.inflateRawSync(data) : null;
    assert(content && content.length === size, `Unsupported or truncated zip entry: ${name}`);
    total += size; assert(total <= 4 * 1024 * 1024 * 1024, 'Archive expands beyond the allowed size');
    const output = path.join(destination, name);
    fs.mkdirSync(path.dirname(output), {recursive: true});
    fs.writeFileSync(output, content);
    if (mode & 0o111) fs.chmodSync(output, 0o755);
  }
}

const tarFilter = (name, entry) => { relative(name.replace(/\/$/, ''), 'archive entry'); return entry.type === 'File' || entry.type === 'Directory'; };

export async function extractInto(archive, format, destination) {
  fs.mkdirSync(destination, {recursive: true});
  if (format === 'zip') return extractZip(archive, destination);
  if (format === 'tar.gz') return tar.x({file: archive, cwd: destination, strict: true, filter: tarFilter});
  if (format === 'tar.xz') {
    const {spawn} = await import('node:child_process');
    const child = spawn('xz', ['-dc', archive], {stdio: ['ignore', 'pipe', 'inherit']});
    const done = new Promise((resolve, reject) => { child.on('error', reject); child.on('close', code => code === 0 ? resolve() : reject(Error(`xz failed (${code}) for ${archive}`))); });
    const extracted = new Promise((resolve, reject) => {
      const stream = tar.x({cwd: destination, strict: true, filter: tarFilter});
      stream.on('error', reject); stream.on('close', resolve); child.stdout.pipe(stream);
    });
    await Promise.all([done, extracted]);
    return undefined;
  }
  throw Error(`Cannot extract ${format}`);
}

// ------------------------------------------------------------------ assembly

const exe = name => process.platform === 'win32' ? `${name}.cmd` : name;

function copyInclude(root, item, directory) {
  const from = typeof item === 'string' ? item : item.from, to = typeof item === 'string' ? item : item.to;
  const source = inside(root, from), target = path.join(directory, relative(to, 'include'));
  fs.mkdirSync(path.dirname(target), {recursive: true});
  fs.copyFileSync(source, target);
}

async function assembleArchive(recipes, pins, id, platform, directory, options) {
  const work = options.work || WORK, pack = recipes.packs[id], spec = pack.platforms[platform], pin = pins.packs[id].platforms[platform];
  const archive = await fetchPinned(pin.url, pin.sha256, path.join(work, 'downloads', pin.sha256), options);
  if (spec.format === 'gz') fs.writeFileSync(path.join(directory, spec.executables[0]), zlib.gunzipSync(fs.readFileSync(archive)));
  else if (spec.format === 'raw') fs.copyFileSync(archive, path.join(directory, spec.executables[0]));
  else {
    const base = path.join(work, 'extracted', pin.sha256), content = path.join(base, 'content');
    if (!fs.existsSync(path.join(base, 'complete'))) {
      removeWork(base, work); await extractInto(archive, spec.format, content); fs.writeFileSync(path.join(base, 'complete'), pin.sha256);
    }
    let root = content;
    const entries = fs.readdirSync(root);
    if (entries.length === 1 && fs.statSync(path.join(root, entries[0])).isDirectory()) root = path.join(root, entries[0]);
    if (spec.include) for (const item of spec.include) copyInclude(root, item, directory);
    else fs.cpSync(root, directory, {recursive: true});
  }
  return `Upstream: ${pin.url}\nSHA-256: ${pin.sha256}\n`;
}

function assembleNpm(recipes, id, directory, options) {
  const work = options.work || WORK, source = npmDirectory(id, options.root || HERE), build = path.join(work, 'npm-build', id);
  removeWork(build, work); fs.mkdirSync(build, {recursive: true});
  for (const name of ['package.json', 'package-lock.json']) fs.copyFileSync(path.join(source, name), path.join(build, name));
  // Lifecycle scripts never run, and the lockfile is the only source of versions.
  run(exe('npm'), ['ci', '--ignore-scripts', '--omit=dev', '--no-audit', '--no-fund', '--no-bin-links'],
    {cwd: build, env: {...process.env, npm_config_update_notifier: 'false'}, shell: process.platform === 'win32'});
  fs.cpSync(path.join(build, 'node_modules'), path.join(directory, 'node_modules'), {recursive: true});
  for (const name of ['package.json', 'package-lock.json']) fs.copyFileSync(path.join(build, name), path.join(directory, name));
  const locked = JSON.parse(fs.readFileSync(path.join(build, 'package-lock.json'), 'utf8')).packages;
  const list = Object.entries(locked).filter(([name, entry]) => name && !entry.link).map(([name, entry]) => `- ${name.replace(/^node_modules\//, '')} ${entry.version} ${entry.integrity}`).sort();
  return `Packages (${list.length}), locked by integrity:\n${list.join('\n')}\n`;
}

function assembleGo(recipes, id, platform, directory, options) {
  const work = options.work || WORK, pack = recipes.packs[id], spec = pack.platforms[platform];
  const gopath = path.join(work, 'gopath', platform), module = `${pack.module}@${pack['module-version']}`;
  const env = {...process.env, GOPATH: gopath, GOBIN: '', GOOS: spec.goos, GOARCH: spec.goarch, CGO_ENABLED: '0',
    GOTOOLCHAIN: `go${pack.go}`, GOFLAGS: '-trimpath', GOWORK: 'off'};
  delete env.GOBIN;
  run('go', ['install', module], {env, cwd: work});
  // A cross-compiled binary lands in a per-target directory.
  const names = spec.executables;
  const found = names.map(name => [path.join(gopath, 'bin', `${spec.goos}_${spec.goarch}`, name), path.join(gopath, 'bin', name)].find(file => fs.existsSync(file)));
  assert(found.every(Boolean), `go install did not produce ${names.join(', ')}`);
  names.forEach((name, i) => fs.copyFileSync(found[i], path.join(directory, name)));
  const info = JSON.parse(run('go', ['mod', 'download', '-json', module], {env: {...env, GOOS: undefined, GOARCH: undefined}, cwd: work}));
  return `Module: ${module}\nModule sum: ${info.Sum}\nGo: ${pack.go}\nTarget: ${spec.goos}/${spec.goarch}\n`;
}

export async function assemblePack(recipes, pins, id, platform, options = {}) {
  const work = options.work || WORK, pack = recipes.packs[id];
  assert(packPlatforms(pack).includes(platform), `${id} is not built for ${platform}`);
  const stage = path.join(work, 'stage', `${id}-${platform}`), directory = path.join(stage, id);
  removeWork(stage, work); fs.mkdirSync(directory, {recursive: true});
  let provenance;
  if (pack.kind === 'archive') provenance = await assembleArchive(recipes, pins, id, platform, directory, options);
  else if (pack.kind === 'npm') provenance = assembleNpm(recipes, id, directory, options);
  else provenance = assembleGo(recipes, id, platform, directory, options);
  for (const license of pack.licenses || []) {
    const pin = pins.packs[id].licenses[license.name];
    const file = await fetchPinned(pin.url, pin.sha256, path.join(work, 'downloads', pin.sha256), options);
    fs.mkdirSync(path.join(directory, 'licenses'), {recursive: true});
    fs.copyFileSync(file, path.join(directory, 'licenses', license.name));
  }
  if (pack.kind !== 'npm') for (const name of pack.platforms[platform].executables) {
    const file = path.join(directory, name);
    assert(fs.existsSync(file) && fs.statSync(file).isFile(), `${id} ${platform}: the archive has no ${name}`);
    fs.chmodSync(file, 0o755);
  }
  fs.writeFileSync(path.join(directory, 'PROVENANCE.md'), `# ${id} ${pack.version}\n\nProduced by Loom (scripts/lsp-servers).\n\nKind: ${pack.kind}\nPlatform: ${platform}\nLicense: ${pack.license}\n${(pack.requires || []).length ? `Requires: ${pack.requires.join(', ')}\n` : ''}\n${provenance}`);
  return directory;
}

export function packRecord(id, platform, archive) {
  return {asset: assetName(id, platform), sha256: fileHash(archive), size: fs.statSync(archive).size};
}

// ------------------------------------------------------------------ catalog

export function serverEntry(server) {
  const entry = {name: server.name, pack: server.pack, command: server.command};
  if (server.args?.length) entry.args = server.args;
  if (server.priority) entry.priority = server.priority;
  if (server.initialization_options) entry.initialization_options = server.initialization_options;
  entry.root = {markers: server.root.markers, ...(server.root.strategy ? {strategy: server.root.strategy} : {}), fallback: server.root.fallback};
  entry.languages = server.languages.map(l => ({language: l.language, ...(l.variant ? {variant: l.variant} : {}), language_id: l.language_id}));
  return entry;
}

/** The catalog after publishing `report`: published packs and their servers are replaced, the rest kept. */
export function makeCatalog(recipes, report, {repository, release, current}) {
  assert(/^[\w.-]+\/[\w.-]+$/.test(repository), 'Invalid release repository');
  assert(RELEASE.test(release), 'Use an lsp-servers-* resource tag');
  assert(report.complete && !report.failures?.length, 'Cannot publish an incomplete batch');
  const packs = {...current.packs}, servers = {...current.servers};
  for (const id of report.selected) {
    const pack = recipes.packs[id], built = report.packs[id];
    assert(built && Object.keys(built.platforms).sort().join() === packPlatforms(pack).sort().join(), `${id} was not built for every platform`);
    packs[id] = {version: pack.version, license: pack.license, ...(pack.requires?.length ? {requires: pack.requires} : {}),
      platforms: Object.fromEntries(Object.entries(built.platforms).map(([platform, record]) => [platform, {asset: record.asset, release, sha256: record.sha256, size: record.size}]))};
    for (const [name, server] of Object.entries(servers)) if (server.pack === id) delete servers[name];
    for (const [name, server] of Object.entries(recipes.servers)) if (server.pack === id) servers[name] = serverEntry(server);
  }
  const sorted = object => Object.fromEntries(Object.keys(object).sort().map(key => [key, object[key]]));
  return validateCatalog({schema: 1, distribution: {repository}, packs: sorted(packs), servers: sorted(servers)});
}

export function validateCatalog(data) {
  assert(data.schema === 1 && /^[\w.-]+\/[\w.-]+$/.test(data.distribution?.repository), 'Invalid publication manifest');
  assert(Object.keys(data).every(key => ['schema', 'distribution', 'packs', 'servers'].includes(key)), 'Build recipes do not belong in the publication manifest');
  const packs = data.packs || {}, servers = data.servers || {};
  for (const [id, pack] of Object.entries(packs)) {
    assert(PACK_ID.test(id) && Object.keys(pack).every(key => ['version', 'license', 'requires', 'platforms'].includes(key)), `Invalid published pack: ${id}`);
    assert(pack.version && pack.license, `Missing pack version or license: ${id}`);
    for (const requirement of pack.requires || []) {
      const [runtime, wanted] = requirement.split('@');
      assert(packs[runtime] && major(packs[runtime].version) === wanted, `${id} requires ${requirement}, which the catalog does not provide`);
    }
    const platforms = Object.keys(pack.platforms || {});
    assert(platforms.length && (!platforms.includes('any') || platforms.length === 1) && platforms.every(p => p === 'any' || PLATFORMS.includes(p)), `Invalid platforms: ${id}`);
    for (const [platform, archive] of Object.entries(pack.platforms)) {
      assert(Object.keys(archive).every(key => ['asset', 'release', 'sha256', 'size'].includes(key)), `Unexpected publication field: ${id}`);
      assert(archive.asset === assetName(id, platform) && RELEASE.test(archive.release), `Invalid release asset: ${id} ${platform}`);
      assert(HASH.test(archive.sha256) && Number.isSafeInteger(archive.size) && archive.size > 0 && archive.size <= MAX_ARCHIVE, `Invalid release digest/size: ${id} ${platform}`);
    }
  }
  for (const [id, server] of Object.entries(servers)) {
    assert(Object.keys(server).every(key => ['name', 'pack', 'command', 'args', 'priority', 'initialization_options', 'root', 'languages'].includes(key)), `Unexpected server field: ${id}`);
    checkServer(id, server, packs);
  }
  return data;
}

export const catalogToml = data => stableToml('# Generated from verified language server artifacts.\n', data);
export function downloadUrl(catalog, id, platform) {
  const archive = catalog.packs[id]?.platforms?.[platform]; assert(archive, `Language server pack is missing: ${id} ${platform}`);
  return `https://github.com/${catalog.distribution.repository}/releases/download/${encodeURIComponent(archive.release)}/${archive.asset}`;
}
export function checkArtifacts(catalog, directory, selected = Object.keys(catalog.packs)) {
  validateCatalog(catalog);
  for (const id of selected) for (const [platform, archive] of Object.entries(catalog.packs[id].platforms)) {
    const file = path.join(directory, archive.asset);
    assert(fs.existsSync(file) && fs.statSync(file).size === archive.size && fileHash(file) === archive.sha256, `Release artifact mismatch: ${id} ${platform}`);
  }
}

// ------------------------------------------------------------------ command line

export function options(argv) {
  const result = {};
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (['--all', '--offline', '--help', '--draft-only', '--refresh'].includes(arg)) result[arg.slice(2)] = true;
    else if (['--packs', '--platforms', '--recipes', '--pins', '--work', '--release', '--repository', '--candidate', '--catalog'].includes(arg)) {
      assert(argv[i + 1] && !argv[i + 1].startsWith('--'), `Missing value: ${arg}`); result[arg.slice(2)] = argv[++i];
    } else throw Error(`Unknown option: ${arg}`);
  }
  assert(!(result.all && result.packs), 'Use either --all or --packs');
  if (result.work) result.work = path.resolve(result.work);
  return result;
}
export function selectPacks(recipes, args) {
  const ids = args.packs ? args.packs.split(',') : Object.keys(recipes.packs);
  assert(ids.length && new Set(ids).size === ids.length, 'Select at least one pack without duplicates');
  ids.forEach(id => assert(recipes.packs[id], `Unknown pack: ${id}`));
  return ids.sort();
}
export function main(callback) { callback().catch(error => { console.error(error.message); process.exitCode = 1; }); }
export const sha256Of = value => createHash('sha256').update(value).digest('hex');
