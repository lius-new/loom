// A workspace log can contain several unit/integration/doc-test summaries.
export function parseTestResults(text) {
  const suites = [...text.matchAll(/test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out/g)];
  if (!suites.length) return null;
  return suites.reduce((total, suite) => ({
    result: total.result === 'ok' && suite[1] === 'ok' ? 'ok' : 'FAILED',
    passed: total.passed + Number(suite[2]),
    failed: total.failed + Number(suite[3]),
    ignored: total.ignored + Number(suite[4]),
    filtered: total.filtered + Number(suite[6]),
  }), { result: 'ok', passed: 0, failed: 0, ignored: 0, filtered: 0 });
}
