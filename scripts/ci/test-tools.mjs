// Run once per validation platform, never as a dependency-setup side effect.
import fs from 'node:fs';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const root = path.resolve(fileURLToPath(new URL('../../',import.meta.url)));
function collect(directory) {
  return fs.readdirSync(directory,{withFileTypes:true}).flatMap(entry => {
    const file = path.join(directory,entry.name);
    return entry.isDirectory() ? entry.name === 'node_modules' ? [] : collect(file) : file.endsWith('.test.mjs') ? [file] : [];
  });
}
const result = spawnSync(process.execPath,['--test',...collect(path.join(root,'scripts')).sort()],{cwd:root,stdio:'inherit',windowsHide:true});
if (result.error) console.error(result.error.message);
process.exitCode = result.status ?? 1;
