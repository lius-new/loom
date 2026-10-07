// Shared helpers for the packaging scripts: project paths, logging, child
// processes, verified downloads, archive extraction and file hashing.
//
// Everything the scripts download or produce lives under `.packaging/` in the
// repository root, which is git-ignored. Nothing is installed system-wide.

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { fileURLToPath } from 'node:url';

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
export const WORK_DIR = path.join(ROOT, '.packaging');
export const DOWNLOADS_DIR = path.join(WORK_DIR, 'downloads');
export const TOOLS_DIR = path.join(WORK_DIR, 'tools');
export const DIST_DIR = path.join(WORK_DIR, 'dist');

export function step(message) {
  console.log(`\n==> ${message}`);
}

export function info(message) {
  console.log(`    ${message}`);
}

export class PackagingError extends Error {}

export function fail(message) {
  throw new PackagingError(message);
}

/** Run a program without a shell, inheriting stdio. Throws on failure. */
export function run(program, args, options = {}) {
  info(`$ ${[program, ...args].map(quoteForLog).join(' ')}`);
  const result = spawnSync(program, args, { stdio: 'inherit', cwd: ROOT, ...options });
  if (result.error) {
    fail(`Could not start ${program}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    fail(`${program} exited with status ${result.status}`);
  }
}

/** Run a program and return its trimmed stdout. Throws on failure. */
export function capture(program, args, options = {}) {
  const result = spawnSync(program, args, { encoding: 'utf8', cwd: ROOT, ...options });
  if (result.error) {
    fail(`Could not start ${program}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    fail(`${program} exited with status ${result.status}\n${result.stderr}`);
  }
  return result.stdout.trim();
}

function quoteForLog(value) {
  return /[\s"]/.test(value) ? JSON.stringify(value) : value;
}

/** A string field of the `[package]` table in Cargo.toml. */
export function cargoPackageField(name) {
  const manifest = fs.readFileSync(path.join(ROOT, 'Cargo.toml'), 'utf8');
  const section = manifest.split(/^\[/m).find((part) => part.startsWith('package]'));
  const match = section && section.match(new RegExp(`^${name}\\s*=\\s*"([^"]+)"`, 'm'));
  if (!match) {
    fail(`Could not read [package] ${name} from Cargo.toml`);
  }
  return match[1];
}

export function sha256File(file) {
  const hash = createHash('sha256');
  const descriptor = fs.openSync(file, 'r');
  try {
    const buffer = Buffer.allocUnsafe(1 << 20);
    let read;
    while ((read = fs.readSync(descriptor, buffer, 0, buffer.length, null)) > 0) {
      hash.update(buffer.subarray(0, read));
    }
  } finally {
    fs.closeSync(descriptor);
  }
  return hash.digest('hex');
}

/**
 * Download `url` into the shared download cache and verify its SHA-256.
 * A cached file is reused only when its hash still matches the pin.
 */
export async function download({ url, file, sha256 }) {
  fs.mkdirSync(DOWNLOADS_DIR, { recursive: true });
  const target = path.join(DOWNLOADS_DIR, file);
  if (fs.existsSync(target)) {
    if (sha256File(target) === sha256) {
      info(`Using cached ${file}`);
      return target;
    }
    info(`Cached ${file} does not match its pinned hash; downloading again`);
    fs.rmSync(target);
  }

  info(`Downloading ${url}`);
  const response = await fetch(url, { redirect: 'follow' });
  if (!response.ok || !response.body) {
    fail(`Download failed (${response.status} ${response.statusText}): ${url}`);
  }
  const partial = `${target}.partial`;
  await pipeline(Readable.fromWeb(response.body), fs.createWriteStream(partial));

  const actual = sha256File(partial);
  if (actual !== sha256) {
    fs.rmSync(partial);
    fail(`SHA-256 mismatch for ${file}\n  expected ${sha256}\n  actual   ${actual}`);
  }
  fs.renameSync(partial, target);
  return target;
}

/** The Windows bsdtar, which reads and writes zip archives. */
function windowsTar() {
  const tar = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe');
  if (!fs.existsSync(tar)) {
    fail(`Windows tar was not found at ${tar}`);
  }
  return tar;
}

/**
 * Extract a zip into `destination` once. Extraction goes to a temporary
 * sibling and is renamed into place, so an interrupted run never leaves a
 * half-populated directory that later looks complete.
 */
export function extractZipOnce(archive, destination) {
  if (fs.existsSync(destination)) {
    info(`Using extracted ${path.relative(ROOT, destination)}`);
    return destination;
  }
  const temporary = `${destination}.partial`;
  fs.rmSync(temporary, { recursive: true, force: true });
  fs.mkdirSync(temporary, { recursive: true });
  run(windowsTar(), ['-xf', archive, '-C', temporary]);
  fs.renameSync(temporary, destination);
  return destination;
}

/** Create a zip whose single top-level entry is `directory`. */
export function createZip(directory, archive) {
  fs.rmSync(archive, { force: true });
  run(windowsTar(), [
    '-a',
    '-c',
    '-f',
    archive,
    '-C',
    path.dirname(directory),
    path.basename(directory),
  ]);
}

/** All files below `directory`, as sorted forward-slash relative paths. */
export function listFiles(directory) {
  const files = [];
  const walk = (current) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (entry.isFile()) {
        files.push(path.relative(directory, full).split(path.sep).join('/'));
      }
    }
  };
  walk(directory);
  return files.sort();
}

export function writeChecksums(files, output) {
  const lines = files.map((file) => `${sha256File(file)}  ${path.basename(file)}`);
  fs.writeFileSync(output, `${lines.join('\n')}\n`);
}
