import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {flags} from './plan.mjs';

export function checkResults(needs) {
  const errors = [];
  for (const job of ['changes','static','core','build']) if (needs[job]?.result !== 'success') errors.push(job + ': ' + (needs[job]?.result || 'missing'));
  const outputs = needs.changes?.outputs || {};
  for (const flag of [...flags,'validation']) if (!['true','false'].includes(outputs[flag])) errors.push('invalid selection output: ' + flag);
  const selected = flags.some(flag => outputs[flag] === 'true');
  if (outputs.validation !== String(selected)) errors.push('inconsistent validation selection');
  const result = needs.validation?.result;
  if (selected ? result !== 'success' : !['success','skipped'].includes(result)) errors.push('validation: ' + (result || 'missing'));
  return errors;
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const errors = checkResults(JSON.parse(process.env.CI_NEEDS));
    if (errors.length) throw Error('CI incomplete: ' + errors.join('; '));
    console.log('All mandatory and selected CI checks passed.');
  } catch (error) {console.error(error.message); process.exitCode = 1;}
}
