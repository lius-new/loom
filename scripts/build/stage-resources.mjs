// Prepare a source build's runtime resources without downloading or compiling grammars.
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {stageLanguagePacks} from '../package/lib/language-packs.mjs';
import {stageLspServers} from '../package/lib/lsp-servers.mjs';

const args = process.argv.slice(2);
if (args.length > 1 || args.some(arg => arg.startsWith('-'))) {
  console.error('Usage: node scripts/build/stage-resources.mjs [executable-directory]');
  process.exitCode = args.length === 1 && args[0] === '--help' ? 0 : 1;
} else {
  const directory = path.resolve(args[0] ?? fileURLToPath(new URL('../../target/debug', import.meta.url)));
  stageLanguagePacks({destination: path.join(directory, 'language-packs')});
  stageLspServers({destination: path.join(directory, 'lsp-servers')});
  console.log(`Prepared syntax resources and the language-server catalog in ${directory}`);
}
