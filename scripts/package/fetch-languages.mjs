// Release input only: download a pinned collection of already compiled standard
// Tree-sitter modules. This script never invokes Cargo or a grammar compiler.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {download, capture, run, WORK_DIR} from './lib/util.mjs';
import {stageLanguagePacks} from './lib/language-packs.mjs';

export function extractLanguageArtifacts(archive, destination, platform = process.platform) {
  const entries = capture('tar', ['-tf', archive]).split(/\r?\n/);
  for (const entry of entries) {
    if (entry.startsWith('/') || entry.includes('\\') || entry.includes(':') || entry.split('/').includes('..')) {
      throw new Error('Invalid language archive path: ' + entry);
    }
  }
  const types = capture('tar', ['-tvf', archive]).split(/\r?\n/);
  if (types.some(line => !/^[-d]/.test(line))) throw new Error('Language archives must contain only files and directories');
  if (fs.existsSync(destination)) throw new Error('Language artifact destination must not exist');
  fs.mkdirSync(path.dirname(destination), {recursive: true});
  const temporary = fs.mkdtempSync(path.join(path.dirname(destination), 'language-input-'));
  try {
    run('tar', ['-xf', archive, '-C', temporary]);
    // Official distributions keep Markdown and JSON enabled, using the same
    // external resource contract as every other language.
    for (const id of ['json', 'markdown']) {
      const manifest = JSON.parse(fs.readFileSync(path.join(temporary, id, 'package.json'), 'utf8'));
      if (manifest.enabled === false) throw new Error('Default language must be enabled: ' + id);
    }
    stageLanguagePacks({source: temporary, destination, platform});
  } catch (error) {
    fs.rmSync(destination, {recursive: true, force: true});
    throw error;
  } finally {
    fs.rmSync(temporary, {recursive: true, force: true});
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const {LOOM_LANGUAGE_PACKS_URL: url, LOOM_LANGUAGE_PACKS_SHA256: sha256} = process.env;
  if (!url || !sha256 || !/^[a-f0-9]{64}$/.test(sha256)) {
    throw new Error('Supply LOOM_LANGUAGE_PACKS_URL and LOOM_LANGUAGE_PACKS_SHA256 for prebuilt language resources; Loom does not build them');
  }
  if (new URL(url).protocol !== 'https:') throw new Error('Language artifact URL must use HTTPS');
  const archive = await download({url, sha256, file: 'languages-' + sha256 + '.tar.gz'});
  extractLanguageArtifacts(archive, path.resolve(process.env.LOOM_LANGUAGE_PACKS || path.join(WORK_DIR, 'language-artifacts')));
}
