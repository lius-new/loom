// Consume complete prebuilt artifacts. This module never builds language code.
import fs from 'node:fs';
import path from 'node:path';

export function moduleFile(module, platform) {
  const extensions = {win32: ['dll', 'wasm'], darwin: ['dylib', 'so', 'wasm'], linux: ['so', 'wasm']};
  const allowed = extensions[platform];
  if (!allowed) throw new Error('Unsupported language module platform: ' + platform);
  if (/^[a-z][a-z0-9_]*$/.test(module)) return module + '.' + allowed[0];
  const match = /^([a-z][a-z0-9_-]*)\.(dll|dylib|so|wasm)$/.exec(module);
  if (!match || !allowed.includes(match[2])) throw new Error('Invalid language module filename for ' + platform);
  return module;
}


export function stageLanguagePacks({ source = process.env.LOOM_LANGUAGE_PACKS, destination, platform = process.platform }) {
  if (!source) throw new Error('Set LOOM_LANGUAGE_PACKS to a complete prebuilt language package collection');
  const root = path.resolve(source);
  const ids = fs.readdirSync(root, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && fs.existsSync(path.join(root, entry.name, 'package.json')))
    .map(entry => entry.name).sort();
  if (!ids.length) throw new Error('No prebuilt language packages supplied');
  const binaries = [];
  for (const id of ids) {
    if (!/^[a-z][a-z0-9_]*$/.test(id)) throw new Error('Invalid language package directory');
    const source = path.join(root, id), target = path.join(destination, id);
    const manifest = JSON.parse(fs.readFileSync(path.join(source, 'package.json'), 'utf8'));
    if (manifest.schema !== 1 || manifest.id !== id || !manifest.grammars) throw new Error('Invalid language package manifest');
    fs.mkdirSync(target, { recursive: true });
    fs.copyFileSync(path.join(source, 'package.json'), path.join(target, 'package.json'));
    const modules = new Set();
    for (const [grammar, entry] of Object.entries(manifest.grammars)) {
      if (!/^[a-z][a-z0-9_]*$/.test(grammar)) throw new Error('Invalid grammar directory');
      moduleFile(entry.module, platform);
      if (entry.symbol !== undefined && !/^[a-z][a-z0-9_]*$/.test(entry.symbol)) throw new Error('Invalid grammar entry point');
      modules.add(entry.module);
      const directory = path.join(target, 'grammars', grammar);
      fs.mkdirSync(directory, { recursive: true });
      for (const file of ['highlights.scm', 'SOURCE.md', 'injections.scm']) {
        const input = path.join(source, 'grammars', grammar, file);
        if (file === 'injections.scm' && !fs.existsSync(input)) continue;
        fs.copyFileSync(input, path.join(directory, file));
      }
    }
    fs.mkdirSync(path.join(target, 'modules'), { recursive: true });
    for (const module of modules) {
      const candidates = /^[a-z][a-z0-9_]*$/.test(module)
        ? ['tree-sitter-' + module + '.wasm', module + '.wasm', moduleFile(module, platform)]
        : [moduleFile(module, platform)];
      const file = candidates.find(file => fs.statSync(path.join(source, 'modules', file), {throwIfNoEntry: false})?.isFile()) || candidates.at(-1);
      const input = path.join(source, 'modules', file);
      if (!fs.existsSync(input)) throw new Error('Missing prebuilt language module: ' + input);
      const output = path.join(target, 'modules', file);
      fs.copyFileSync(input, output);
      binaries.push(output);
    }
  }
  return binaries;
}
