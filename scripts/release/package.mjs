// One ordered build per platform; final artifact checks stay in the workflow.
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
import {ROOT, validateVersion} from './notes.mjs';
import {stampVersion} from './version.mjs';
export function packagePlan(platform, version) {
  if (!['windows','linux','macos'].includes(platform)) throw Error('Unsupported release platform');
  validateVersion(version);
  return ['setup','compile','package','bundle'].map(step => [path.join(ROOT,'scripts/package/package.mjs'),step,'--platform',platform,'--expect-version',version]);
}
export function packageRelease({platform, version, sha, root = ROOT}, {run = execFileSync, stamp = stampVersion} = {}) {
  const plan = packagePlan(platform,version);
  stamp({root,version,sha});
  for (const args of plan) run(process.execPath,args,{cwd:ROOT,stdio:'inherit',env:process.env,windowsHide:true});
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {packageRelease({platform:process.env.PLATFORM,version:process.env.VERSION,sha:process.env.RELEASE_SHA});}
  catch (error) {console.error(error.message);process.exitCode = 1;}
}
