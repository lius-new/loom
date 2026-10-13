import test from 'node:test';
import assert from 'node:assert/strict';
import { parseTestResults } from './test-results.mjs';

test('workspace unit and integration results survive a final empty doctest suite', () => {
  const summary = parseTestResults([
    'test result: ok. 517 passed; 0 failed; 12 ignored; 0 measured; 0 filtered out',
    'test result: ok. 65 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out',
    'test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out',
    'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out',
  ].join('\n'));
  assert.deepEqual(summary, { result: 'ok', passed: 585, failed: 0, ignored: 12, filtered: 0 });
});
test('a failed earlier suite cannot be hidden by later successful suites', () => {
  const summary = parseTestResults([
    'test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out',
    'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 10 filtered out',
  ].join('\n'));
  assert.equal(summary.result, 'FAILED');
  assert.equal(summary.failed, 1);
  assert.equal(summary.filtered, 10);
});
test('a build failure without test results is not counted as a passing suite', () => {
  assert.equal(parseTestResults('error: could not compile core'), null);
});
