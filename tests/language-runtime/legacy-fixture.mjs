// Compatibility test data only. Convert supplied standard resources into old
// manifest fixtures so migration/error tests can still mutate the legacy shape.
// No parser source, compiler, generated module or application resource is built.
import fs from 'node:fs';
import path from 'node:path';

export function legacyFixture(input, destination) {
  const defaults = JSON.parse(fs.readFileSync(new URL('../../src/infrastructure/language_defaults.json', import.meta.url)));
  fs.rmSync(destination, {recursive: true, force: true});
  fs.mkdirSync(destination, {recursive: true});
  const collections = {};
  for (const entry of fs.readdirSync(input, {withFileTypes: true}).filter(entry => entry.isDirectory())) {
    const root = path.join(input, entry.name), standard = path.join(root, 'tree-sitter.json');
    if (!fs.existsSync(standard)) {
      if (fs.existsSync(path.join(root, 'package.json'))) {
        fs.cpSync(root, path.join(destination, entry.name), {recursive: true});
        collections[entry.name] = root;
      }
      continue;
    }
    const document = JSON.parse(fs.readFileSync(standard)), names = new Set(document.grammars.map(g => g.name));
    const languages = structuredClone(defaults.languages.filter(d => d.variants.some(v => names.has(v.grammar))));
    const first = document.grammars[0].name;
    const id = languages.find(d => d.variants.some(v => v.grammar === first))?.name ?? first;
    const target = path.join(destination, id), grammars = {};
    collections[id] = root;
    for (const language of languages) {
      language.variants = language.variants.filter(v => names.has(v.grammar));
      language.files = document.grammars.filter(g => language.variants.some(v => v.grammar === g.name) && g['file-types']?.length)
        .map(g => ({extensions: g['file-types'], variant: language.variants.find(v => v.grammar === g.name).id}));
    }
    const servers = [];
    fs.mkdirSync(path.join(target, 'modules'), {recursive: true});
    for (const grammar of document.grammars) {
      const name = grammar.name, extension = {win32: 'dll', darwin: 'dylib', linux: 'so'}[process.platform];
      const candidates = ['tree-sitter-' + name + '.wasm', name + '.wasm', name + '.' + extension, 'libtree-sitter-' + name + '.' + extension, 'parser.' + extension]
        .flatMap(file => [path.join(root, file), path.join(root, grammar.path ?? '.', file)]);
      const module = candidates.find(file => fs.statSync(file, {throwIfNoEntry: false})?.isFile());
      if (!module) throw new Error('Native regression collection is incomplete: ' + name);
      const file = name + path.extname(module);
      fs.copyFileSync(module, path.join(target, 'modules', file));
      grammars[name] = {module: name};
      const queryRoot = path.join(target, 'grammars', name);
      fs.mkdirSync(queryRoot, {recursive: true});
      for (const kind of ['highlights', 'injections']) {
        const paths = [grammar[kind] ?? 'queries/' + kind + '.scm'].flat();
        const text = paths.filter(file => kind === 'highlights' || fs.existsSync(path.join(root, file))).map(file => fs.readFileSync(path.join(root, file), 'utf8')).join('\n');
        if (text) fs.writeFileSync(path.join(queryRoot, kind + '.scm'), text);
      }
    }
    fs.writeFileSync(path.join(target, 'package.json'), JSON.stringify({schema: 1, id, languages, servers, grammars}));
  }
  return {root: path.resolve(destination), collections};
}
