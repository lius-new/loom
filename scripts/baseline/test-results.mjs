// Keep Cargo's suite boundaries so workspace validation is not counted as
// additional application coverage. Empty doctests remain valid suites.
export function parseTestSuites(text) {
  const suites = [];
  let context = {crate: 'unknown', target: 'unknown', kind: 'unknown'}, tests = [], pending;
  for (const line of text.replace(/\x1b\[[0-9;]*m/g, '').split(/\r?\n/)) {
    const running = /^\s*Running (.+) \((.+)\)\s*$/.exec(line);
    const docs = /^\s*Doc-tests\s+(\S+)/.exec(line);
    if (running) {
      const binary = running[2].replaceAll('\\', '/').split('/').at(-1).replace(/(?:-[0-9a-f]+)?(?:\.exe)?$/, '');
      context = {crate: binary.replaceAll('_', '-').toLowerCase(), target: running[1], kind: running[1].startsWith('unittests ') ? 'unit' : 'integration'};
      tests = [];
      pending = undefined;
    } else if (docs) {
      context = {crate: docs[1].replaceAll('_', '-'), target: docs[1], kind: 'doc'};
      tests = [];
      pending = undefined;
    }
    const test = /^test (\S+) \.\.\. (.*)$/.exec(line);
    if (test) pending = test[1];
    const status = /^(ok|FAILED|ignored)(?:[, ](.*))?$/.exec(test ? test[2] : line);
    if (pending && status) {
      tests.push({name: pending, result: status[1], reason: status[2] || ''});
      pending = undefined;
    }
    const summary = /test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out/.exec(line);
    if (summary) {
      suites.push({...context, result: summary[1], passed: Number(summary[2]), failed: Number(summary[3]),
        ignored: Number(summary[4]), filtered: Number(summary[6]), tests});
      tests = [];
      pending = undefined;
    }
  }
  return suites;
}

export function parseTestResults(text) {
  const suites = parseTestSuites(text);
  if (!suites.length) return null;
  return suites.reduce((total, suite) => ({
    result: total.result === 'ok' && suite.result === 'ok' ? 'ok' : 'FAILED',
    passed: total.passed + suite.passed,
    failed: total.failed + suite.failed,
    ignored: total.ignored + suite.ignored,
    filtered: total.filtered + suite.filtered,
  }), {result: 'ok', passed: 0, failed: 0, ignored: 0, filtered: 0});
}
