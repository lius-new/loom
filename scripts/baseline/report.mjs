import fs from 'node:fs';
import path from 'node:path';
import { parseTestResults } from './test-results.mjs';

const directory = path.resolve(process.argv[2] || '');
const logs = path.join(directory, 'logs');
const rows = [];
const checks = [];
const notices = [];
const diagnostics = new Map();
let testInventory = [];
function milliseconds(value) {
  const found = /^([\d.]+)(ns|µs|us|ms|s)$/.exec(value);
  if (!found) throw new Error('Unknown duration: ' + value);
  return Number(found[1]) * ({ ns: 0.000001, 'µs': 0.001, us: 0.001, ms: 1, s: 1000 })[found[2]];
}
function add(scenario, metric, value, unit, sample, log, extra = {}) {
  rows.push({ scenario, metric, value, unit, sample, log, ...extra });
}
for (const name of fs.readdirSync(logs).filter(name => name.endsWith('.log')).sort()) {
  const text = fs.readFileSync(path.join(logs, name), 'utf8').replace(/^\uFEFF/, '');
  const outcome = parseTestResults(text);
  if (outcome) checks.push({ log: name, ...outcome });
  if (name === 'tests-isolated.log') {
    testInventory = [...text.matchAll(/^test (\S+) \.\.\. (ok|FAILED|ignored)(.*)$/gm)].map(match => ({
      name: match[1], result: match[2], reason: match[3].replace(/^[, ]+/, '').trim()
    }));
  }
  if (name === 'clippy.log') {
    for (const match of text.matchAll(/error: ([^\r\n]+)\r?\n\s+--> ((?:src|crates)[\\/][^:\r\n]+):(\d+):(\d+)/g)) {
      const file = match[2].replaceAll('\\', '/');
      diagnostics.set(file + ':' + match[3], { message: match[1], file, line: Number(match[3]), column: Number(match[4]) });
    }
  }
  const numbered = /-(\d+)\.log$/.exec(name);
  if (!numbered || Number(numbered[1]) === 0) continue;
  // Failed tests are invalid samples even if they printed a timing.
  if (!outcome || outcome.result !== 'ok') {
    notices.push({ log: name, reason: 'invalid performance sample' });
    continue;
  }
  const sample = Number(numbered[1]);
  for (const line of text.split(/\r?\n/)) {
    const marker = line.indexOf('LOOM_BASELINE ');
    if (marker >= 0) {
      const record = JSON.parse(line.slice(marker + 'LOOM_BASELINE '.length));
      rows.push({ ...record, sample, log: name });
    }
    if (line.includes('LOOM_BASELINE_STATUS')) notices.push({ log: name, reason: line.trim() });
    let match = /(\d+) MB: typing (\S+)\/char, paste 1 MB (\S+), undo (\S+), select-all indent (\S+), history ([\d.]+) MB/.exec(line);
    if (match) {
      const scenario = 'ascii-' + match[1] + 'm';
      ['typing_per_char', 'paste_1m', 'undo_paste', 'select_all_indent'].forEach((metric, index) =>
        add(scenario, metric, milliseconds(match[index + 2]), 'ms', sample, name));
      add(scenario, 'history_bytes_rounded', Number(match[6]) * 1024 * 1024, 'bytes', sample, name);
    }
    match = /(8 MiB single line|Unicode single line|many short lines): ([\d.]+) MiB, prepare (\S+), 100 viewport renders and clicks (\S+)/.exec(line);
    if (match) {
      add(match[1], 'ui_document_prepare', milliseconds(match[3]), 'ms', sample, name);
      add(match[1], 'ui_100_viewports_and_clicks', milliseconds(match[4]), 'ms', sample, name);
    }
    match = /(8 MiB single line|Unicode single line|many short lines): 20 end-of-file edits and renders (\S+)/.exec(line);
    if (match) add(match[1], 'ui_20_edits_and_renders', milliseconds(match[2]), 'ms', sample, name);
    match = /(100|500) tabs: (\S+) per pointer update/.exec(line);
    if (match) add(match[1] + '-tabs', 'pointer_update', milliseconds(match[2]), 'ms', sample, name);
  }
}
const groups = new Map();
for (const row of rows) {
  const key = row.scenario + '/' + row.metric;
  if (!groups.has(key)) groups.set(key, []);
  groups.get(key).push(row);
}
const statistics = [];
for (const records of groups.values()) {
  const values = records.map(row => row.value).sort((a, b) => a - b);
  const middle = Math.floor(values.length / 2);
  const median = values.length % 2 ? values[middle] : (values[middle - 1] + values[middle]) / 2;
  statistics.push({ scenario: records[0].scenario, metric: records[0].metric, samples: values.length,
    unit: records[0].unit, median, min: values[0], max: values.at(-1) });
}
fs.writeFileSync(path.join(directory, 'measurements.json'), JSON.stringify({ schema: 1, records: rows, statistics, checks, notices }, null, 2) + '\n');
if (testInventory.length) fs.writeFileSync(path.join(directory, 'test-inventory.json'), JSON.stringify(testInventory, null, 2) + '\n');
fs.writeFileSync(path.join(directory, 'clippy-diagnostics.json'), JSON.stringify([...diagnostics.values()], null, 2) + '\n');
const escape = value => '"' + String(value).replaceAll('"', '""') + '"';
const csv = ['scenario,metric,samples,unit,median,min,max',
  ...statistics.map(row => [row.scenario, row.metric, row.samples, row.unit, row.median, row.min, row.max].map(escape).join(','))].join('\n') + '\n';
fs.writeFileSync(path.join(directory, 'performance.csv'), '\uFEFF' + csv);
const lines = ['# 阶段零自动采样结果', '', '样本零为预热，已排除。以下为同机 release 测试中位数和范围；UI session 数据不等于原生窗口帧率。', '',
  '| 场景 | 指标 | 样本 | 单位 | 中位数 | 最小值 | 最大值 |', '| --- | --- | --- | --- | --- | --- | --- |'];
for (const row of statistics) lines.push('| ' + [row.scenario, row.metric, row.samples, row.unit, row.median.toFixed(6), row.min.toFixed(6), row.max.toFixed(6)].join(' | ') + ' |');
lines.push('', '## 测试日志结果', '', '| 日志 | 结果 | 通过 | 失败 | 忽略 |', '| --- | --- | --- | --- | --- |');
for (const row of checks.filter(row => !/-\d+\.log$/.test(row.log))) lines.push('| ' + [row.log, row.result, row.passed, row.failed, row.ignored].join(' | ') + ' |');
if (notices.length) {
  lines.push('', '## 采样说明', '');
  for (const item of notices) lines.push('- ' + item.log + ': ' + item.reason);
}
fs.writeFileSync(path.join(directory, 'measurements.md'), lines.join('\n') + '\n');
console.log('Recorded ' + rows.length + ' measurements across ' + statistics.length + ' metrics.');
