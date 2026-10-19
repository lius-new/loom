// Consume complete prebuilt artifacts. This module never builds language code.
import fs from 'node:fs';
import path from 'node:path';

function relative(file) {
  if (typeof file !== 'string' || !file || file.startsWith('/') || /[\\:\0]/.test(file) || file.split('/').includes('..')) throw new Error('Invalid standard resource path: ' + file);
  return file;
}
function contained(source, file) {
  const root = fs.realpathSync(source), resolved = fs.realpathSync(path.join(source, relative(file)));
  if (!resolved.startsWith(root + path.sep)) throw new Error('Standard resource escapes language directory: ' + file);
  return resolved;
}
function stageStandard({source, target, platform}) {
  const input = contained(source, 'tree-sitter.json');
  if (fs.statSync(input).size > 1024 * 1024) throw new Error('Standard manifest exceeds byte limit');
  const manifest = JSON.parse(fs.readFileSync(input, 'utf8'));
  if (!Array.isArray(manifest.grammars) || !manifest.grammars.length || manifest.grammars.length > 128) throw new Error('Invalid standard grammars');
  const copied = new Set(), binaries = [], names = new Set();
  const copy = file => {
    file = relative(file);
    const input = contained(source, file), output = path.join(target, file);
    if (!copied.has(output)) { fs.mkdirSync(path.dirname(output), {recursive: true}); fs.copyFileSync(input, output); copied.add(output); }
    return output;
  };
  copy('tree-sitter.json');
  for (const grammar of manifest.grammars) {
    const name = grammar.name;
    if (typeof name !== 'string' || !/^[a-z][a-z0-9_]*$/.test(name) || name === 'plaintext' || names.has(name)) throw new Error('Invalid or duplicate standard grammar name');
    names.add(name);
    const base = relative(grammar.path ?? '.');
    for (const kind of ['highlights', 'injections', 'locals']) {
      const value = grammar[kind] ?? 'queries/' + kind + '.scm';
      const files = typeof value === 'string' ? [value] : value;
      if (!Array.isArray(files) || files.length > 128) throw new Error('Invalid standard query paths');
      for (const file of files) {
        relative(file);
        if (kind !== 'highlights' && !fs.existsSync(path.join(source, file))) continue;
        copy(file);
      }
    }
    const extension = {win32: 'dll', linux: 'so', darwin: 'dylib'}[platform];
    if (!extension) throw new Error('Unsupported language module platform');
    const candidates = ['tree-sitter-' + name + '.wasm', name + '.wasm', name + '.' + extension, 'libtree-sitter-' + name + '.' + extension, 'parser.' + extension]
      .flatMap(file => [file, path.posix.join(base, file)]);
    const file = candidates.find(file => fs.statSync(path.join(source, file), {throwIfNoEntry: false})?.isFile());
    if (!file) throw new Error('Missing prebuilt language module: ' + name);
    const output = copy(file);
    if (!binaries.includes(output)) binaries.push(output);
  }
  // Standard licenses and optional provenance are distribution data, never a
  // prerequisite for discovery or highlighting.
  for (const file of ['LICENSE', 'LICENSE.md', 'LICENSE.txt', 'COPYING', 'PROVENANCE.md']) {
    if (fs.statSync(path.join(source, file), {throwIfNoEntry: false})?.isFile()) copy(file);
  }
  return binaries;
}

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
    .filter(entry => entry.isDirectory() && ['tree-sitter.json', 'package.json'].some(file => fs.existsSync(path.join(root, entry.name, file))))
    .map(entry => entry.name).sort();
  if (!ids.length) throw new Error('No prebuilt language packages supplied');
  const binaries = [];
  for (const id of ids) {
    if (!/^[a-zA-Z0-9_-]+$/.test(id)) throw new Error('Invalid language package directory');
    const source = path.join(root, id), target = path.join(destination, id);
    if (fs.existsSync(path.join(source, 'tree-sitter.json'))) {
      binaries.push(...stageStandard({source, target, platform}));
      continue;
    }
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
        if (file !== 'highlights.scm' && !fs.existsSync(input)) continue;
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
