import fs from 'node:fs';
import {archive, sha256, tar} from './cache.mjs';

export function skiaSpec(manifest, target, lock, graph) {
  const spec = manifest.skia.targets[target];
  if (!spec) throw Error(`Unsupported Skia target: ${target}`);
  const locked = lock.match(/\bname = "skia-bindings"\r?\nversion = "([^"]+)"/);
  if (locked?.[1] !== manifest.skia.version) throw Error('Skia manifest does not match Cargo.lock; update pinned dependency inputs.');
  if (graph.trim() !== `skia-bindings v${manifest.skia.version}|${spec.features}`) throw Error(`Skia feature set changed for ${target}; update pinned dependency inputs. Resolved: ${graph.trim()}`);
  return spec;
}

export async function prepareSkia(spec, options) {
  const file = options.skiaArchive || await archive(spec, options);
  if (!fs.existsSync(file) || await sha256(file) !== spec.sha256) throw Error(`Skia archive missing or SHA-256 mismatch: ${file}`);
  const key = tar(['-xOf', file, 'skia-binaries/key.txt']).trim();
  const version = tar(['-xOf', file, 'skia-binaries/tag.txt']).trim();
  if (key !== spec.key || version !== options.manifest.skia.version) throw Error(`Skia archive version/key mismatch: ${file}`);
  // rust-skia strips file:// and reads the remaining native path; do not URI
  // encode spaces or use WHATWG file:///C:/... on Windows.
  return {archive: file, url: 'file://' + file.replaceAll('\\', '/')};
}
