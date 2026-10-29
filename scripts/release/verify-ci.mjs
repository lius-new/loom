import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {github} from './github.mjs';
import {verifySource} from './publish.mjs';

export const requiredChecks = ['CI result','Format and architecture','Independent editor and workspace rules',
  'Test Linux x64','Test Windows x64','Test macOS host'];
async function pages(endpoint, field, run) {
  const result = await run(['api','--method','GET',endpoint,'--paginate','--slurp']);
  if (result.status !== 0) throw Error('Cannot verify CI: ' + (result.stderr || result.stdout));
  const data = JSON.parse(result.stdout);
  const responses = Array.isArray(data) ? data : [data];
  if (!responses.every(page => Array.isArray(page[field]))) throw Error('Malformed CI API response');
  return responses.flatMap(page => page[field]);
}
export async function verifyCi({repo, sha}, run = github) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo) || !/^[a-f0-9]{40}(?:[a-f0-9]{24})?$/i.test(sha)) throw Error('Invalid release repository or commit');
  const runs = await pages('repos/' + repo + '/actions/workflows/ci.yml/runs?head_sha=' + sha + '&per_page=100','workflow_runs',run);
  const candidates = runs.filter(item => item.head_sha === sha && item.head_repository?.full_name === repo &&
    ['push','workflow_dispatch'].includes(item.event));
  candidates.sort((a,b) => b.run_number - a.run_number || b.id - a.id);
  const latest = candidates[0];
  if (!latest || latest.status !== 'completed' || latest.conclusion !== 'success') {
    throw Error('Release requires completed successful CI for exact commit ' + sha + '. Finish or rerun CI, then rerun this release; no builds started.');
  }
  const jobs = await pages('repos/' + repo + '/actions/runs/' + latest.id + '/jobs?filter=latest&per_page=100','jobs',run);
  const missing = requiredChecks.filter(name => !jobs.some(job => job.name === name && job.conclusion === 'success' && job.head_sha === sha));
  if (missing.length) throw Error('Release CI verification missing: ' + missing.join(', '));
  return {id:latest.id,url:latest.html_url};
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const version = process.env.INPUT_VERSION || (process.env.GITHUB_REF_NAME || '').replace(/^v/,'');
    const repo = process.env.GITHUB_REPOSITORY;
    const sha = verifySource({version,target:'HEAD',repo});
    const ci = await verifyCi({repo,sha});
    fs.appendFileSync(process.env.GITHUB_OUTPUT,'version=' + version + '\nsha=' + sha + '\n');
    if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY,'Release commit: ' + sha + '\n\nVerified CI run: ' + ci.id + '\n');
    console.log('Verified release ' + version + ' at ' + sha + ' using CI run ' + ci.id);
  } catch (error) {console.error(error.message);process.exitCode = 1;}
}
