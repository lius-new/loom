// Stage independent packages, including their native grammars.
import fs from 'node:fs';
import path from 'node:path';

export function moduleFile(module, platform) {
  if (!/^[a-z][a-z0-9_]*$/.test(module)) throw new Error('Invalid language module name');
  if (platform === 'win32') return module + '.dll';
  if (platform === 'darwin') return 'lib' + module + '.dylib';
  if (platform === 'linux') return 'lib' + module + '.so';
  throw new Error('Unsupported language module platform: ' + platform);
}


function manifests(profile) {
  const root = path.join(profile, 'language-packs');
  return fs.readdirSync(root, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && fs.existsSync(path.join(root, entry.name, 'package.json')))
    .map(entry => entry.name).sort();
}

export function stageLanguagePacks({ profiles, destination, platform = process.platform, combine }) {
  if (!profiles.length) throw new Error('No language build profile supplied');
  const packages = manifests(profiles[0]);
  if (!packages.length) throw new Error('No independent language packages built');
  for (const profile of profiles.slice(1)) {
    if (JSON.stringify(manifests(profile)) !== JSON.stringify(packages)) throw new Error('Language packages differ between target architectures');
  }
  const binaries = [];
  for (const id of packages) {
    if (!/^[a-z][a-z0-9_]*$/.test(id)) throw new Error('Invalid language package directory');
    const source = path.join(profiles[0], 'language-packs', id);
    const text = fs.readFileSync(path.join(source, 'package.json'), 'utf8');
    const manifest = JSON.parse(text);
    if (manifest.schema !== 1 || manifest.id !== id || !manifest.grammars) throw new Error('Invalid language package manifest');
    for (const profile of profiles.slice(1)) {
      if (fs.readFileSync(path.join(profile, 'language-packs', id, 'package.json'), 'utf8') !== text) {
        throw new Error('Language manifests differ between target architectures');
      }
    }
    const target = path.join(destination, id);
    fs.mkdirSync(target, { recursive: true });
    fs.copyFileSync(path.join(source, 'package.json'), path.join(target, 'package.json'));
    const modules = new Set();
    for (const [grammar, entry] of Object.entries(manifest.grammars)) {
      if (!/^[a-z][a-z0-9_]*$/.test(grammar)) throw new Error('Invalid grammar directory');
      moduleFile(entry.module, platform);
      if (!/^[a-z][a-z0-9_]*$/.test(entry.symbol)) throw new Error('Invalid grammar entry point');
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
      const file = moduleFile(module, platform);
      const inputs = profiles.map(profile => path.join(profile, file));
      for (const input of inputs) {
        if (!fs.existsSync(input)) throw new Error('Missing language module; build the workspace first: ' + input);
      }
      const output = path.join(target, 'modules', file);
      if (inputs.length > 1) {
        if (!combine) throw new Error('Multiple architectures require a module combiner');
        combine(inputs, output);
      } else {
        fs.copyFileSync(inputs[0], output);
      }
      binaries.push(output);
    }
  }
  return binaries;
}
