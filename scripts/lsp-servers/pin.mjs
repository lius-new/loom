// Resolves the pins for the recipes: the SHA-256 of every upstream archive and license
// file, and the lockfile of every npm pack. Run it after changing a version in
// build.toml, then review and commit pins.toml and npm/. Nothing is fetched at build
// time that was not pinned here.
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {Readable} from 'node:stream';
import {pipeline} from 'node:stream/promises';
import {fileURLToPath} from 'node:url';
import {PINS, RECIPES, HERE, assert, loadRecipes, loadPins, assetUrl, npmDirectory, stableToml, writeAtomic, run,
  options, main, selectPacks, HASH} from './lib.mjs';

const headers = () => ({'User-Agent': 'loom-lsp-servers', ...(process.env.GITHUB_TOKEN ? {Authorization: `Bearer ${process.env.GITHUB_TOKEN}`} : {})});

async function digestOf(url) {
  const response = await fetch(url, {redirect: 'follow', headers: headers(), signal: AbortSignal.timeout(600000)});
  assert(response.ok && response.body, `Download failed (${response.status}): ${url}`);
  const hasher = createHash('sha256');
  let size = 0;
  await pipeline(Readable.fromWeb(response.body), async function* (source) { for await (const chunk of source) { hasher.update(chunk); size += chunk.length; yield chunk; } }, async function (source) { for await (const chunk of source) void chunk; });
  return {sha256: hasher.digest('hex'), size};
}

const releases = new Map();
async function githubAssets(repository, tag) {
  const key = `${repository}@${tag}`;
  if (!releases.has(key)) {
    const response = await fetch(`https://api.github.com/repos/${repository}/releases/tags/${encodeURIComponent(tag)}`, {headers: headers()});
    assert(response.ok, `GitHub release not found (${response.status}): ${key}`);
    releases.set(key, new Map((await response.json()).assets.map(asset => [asset.name, asset])));
  }
  return releases.get(key);
}
const nodeSums = new Map();
async function nodeChecksums(version) {
  if (!nodeSums.has(version)) {
    const response = await fetch(`https://nodejs.org/dist/v${version}/SHASUMS256.txt`);
    assert(response.ok, `Node release not found: ${version}`);
    nodeSums.set(version, new Map((await response.text()).trim().split(/\r?\n/).map(line => { const [sum, name] = line.trim().split(/\s+/); return [name, sum]; })));
  }
  return nodeSums.get(version);
}

/** The pin for one archive: from the publisher's own checksum when it gives one, else by hashing the download. */
async function pinArchive(pack, platform) {
  const url = assetUrl(pack, platform), spec = pack.platforms[platform];
  if (pack.upstream.source === 'nodejs') {
    const sum = (await nodeChecksums(pack.version)).get(spec.asset);
    assert(HASH.test(sum || ''), `No published checksum for ${spec.asset}`);
    const head = await fetch(url, {method: 'HEAD'});
    return {url, sha256: sum, size: Number(head.headers.get('content-length'))};
  }
  const asset = (await githubAssets(pack.upstream.repository, pack.upstream.tag)).get(spec.asset);
  assert(asset, `The release has no asset ${spec.asset}`);
  const digest = asset.digest?.replace(/^sha256:/, '');
  if (HASH.test(digest || '')) return {url, sha256: digest, size: asset.size};
  const measured = await digestOf(url);
  assert(measured.size === asset.size, `Downloaded size differs from the release: ${spec.asset}`);
  return {url, ...measured};
}

export async function pinRecipes(recipes, existing, {ids = Object.keys(recipes.packs), refresh = false, log = console.log} = {}) {
  const pins = {schema: 1, packs: {}};
  for (const [id, pack] of Object.entries(recipes.packs)) {
    if (pack.kind === 'npm') continue;
    const archives = pack.kind === 'archive';
    const old = existing.packs?.[id], entry = {...(archives ? {platforms: {}} : {}), ...((pack.licenses || []).length ? {licenses: {}} : {})};
    for (const platform of archives ? Object.keys(pack.platforms) : []) {
      const url = assetUrl(pack, platform), kept = old?.platforms?.[platform];
      if (kept && kept.url === url && (!refresh || !ids.includes(id))) { entry.platforms[platform] = kept; continue; }
      assert(ids.includes(id), `${id} ${platform} needs a pin; include ${id} in --packs`);
      log(`pinning ${id} ${platform}`);
      entry.platforms[platform] = await pinArchive(pack, platform);
    }
    for (const license of pack.licenses || []) {
      const kept = old?.licenses?.[license.name];
      if (kept && kept.url === license.url && (!refresh || !ids.includes(id))) { entry.licenses[license.name] = kept; continue; }
      assert(ids.includes(id), `${id} ${license.name} needs a pin; include ${id} in --packs`);
      log(`pinning ${id} ${license.name}`);
      entry.licenses[license.name] = {url: license.url, ...(await digestOf(license.url))};
    }
    pins.packs[id] = entry;
  }
  return pins;
}

/** Writes package.json and the lockfile of an npm pack; the lockfile pins every dependency by integrity. */
export function pinNpm(id, pack, root = HERE) {
  const directory = npmDirectory(id, root);
  fs.mkdirSync(directory, {recursive: true});
  const dependencies = Object.fromEntries(pack.packages.map(spec => { const at = spec.lastIndexOf('@'); return [spec.slice(0, at), spec.slice(at + 1)]; }));
  const manifest = {name: `loom-lsp-${id}`, version: '0.0.0', private: true, dependencies};
  writeAtomic(path.join(directory, 'package.json'), JSON.stringify(manifest, null, 2) + '\n');
  fs.rmSync(path.join(directory, 'package-lock.json'), {force: true});
  run(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['install', '--package-lock-only', '--ignore-scripts', '--no-audit', '--no-fund'],
    {cwd: directory, shell: process.platform === 'win32', env: {...process.env, npm_config_update_notifier: 'false'}});
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main(async () => {
  const args = options(process.argv.slice(2));
  if (args.help) { console.log('node scripts/lsp-servers/pin.mjs [--packs rust-analyzer,pyright] [--refresh]'); return; }
  const recipes = loadRecipes(args.recipes || RECIPES), ids = selectPacks(recipes, args);
  const pinsFile = args.pins || PINS;
  const pins = await pinRecipes(recipes, loadPins(pinsFile), {ids, refresh: args.refresh});
  writeAtomic(pinsFile, stableToml('# Written by scripts/lsp-servers/pin.mjs. Do not edit by hand.\n', pins));
  for (const id of ids) if (recipes.packs[id].kind === 'npm' && (args.refresh || !fs.existsSync(path.join(npmDirectory(id), 'package-lock.json')))) { console.log(`locking ${id}`); pinNpm(id, recipes.packs[id]); }
});
