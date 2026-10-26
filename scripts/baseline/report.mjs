import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {evaluateReport} from './report-core.mjs';

const readJson = file => fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, '')) : undefined;
const cell = value => String(value ?? '').replaceAll('|', '\\|').replace(/[\r\n]+/g, ' ');
const csv = value => '"' + String(value).replaceAll('"', '""') + '"';

export function generateReport(directory) {
  directory = path.resolve(directory);
  const logDirectory = path.join(directory, 'logs');
  const logs = Object.fromEntries(fs.readdirSync(logDirectory).filter(name => name.endsWith('.log'))
    .sort().map(name => [name, fs.readFileSync(path.join(logDirectory, name), 'utf8').replace(/^\uFEFF/, '')]));
  let recorded = readJson(path.join(directory, 'checks.json'));
  if (recorded && !Array.isArray(recorded)) recorded = [recorded]; // Old PowerShell single-object output.
  const report = evaluateReport({logs, checks: recorded, environment: readJson(path.join(directory, 'environment.json'))});
  const writeJson = (name, value) => fs.writeFileSync(path.join(directory, name), JSON.stringify(value, null, 2) + '\n');
  writeJson('measurements.json', report);
  writeJson('test-inventory.json', report.inventory);
  writeJson('clippy-diagnostics.json', report.diagnostics);
  const columns = ['scenario', 'metric', 'samples', 'unit', 'median', 'min', 'max'];
  fs.writeFileSync(path.join(directory, 'performance.csv'), '\uFEFF' + [columns.join(','),
    ...report.statistics.map(row => columns.map(key => csv(row[key])).join(','))].join('\n') + '\n');
  const lines = ['# 回归与性能采样结果', '',
    `完整性：${report.complete ? '通过' : '不完整或失败'}；退出码记录：${report.verified ? '已核验' : '缺失，历史日志未核验'}。`, '',
    '样本 0 为预热，不计入统计。结果仅适合同机、相同工具链和测量条件比较；UI session 不代表原生窗口帧率。', '',
    '| 场景 | 指标 | 样本 | 单位 | 中位数 | 最小值 | 最大值 |', '| --- | --- | --- | --- | --- | --- | --- |'];
  for (const row of report.statistics) lines.push('| ' + columns.map(key => cell(row[key])).join(' | ') + ' |');
  lines.push('', '## 检查结果', '', '| 日志 | 结果 | 退出码 | 通过 | 失败 | 忽略 |', '| --- | --- | --- | --- | --- | --- |');
  for (const row of report.checks) lines.push('| ' + [row.log, row.result, row.exit_code, row.passed, row.failed, row.ignored].map(cell).join(' | ') + ' |');
  lines.push('', '## 按 crate 与套件统计', '',
    'workspace 验证 crate 复用生产规则，其测试数量不代表新增独立行为覆盖。', '',
    '| 日志 | crate | 类型 | 目标 | 通过 | 失败 | 忽略 |', '| --- | --- | --- | --- | --- | --- | --- |');
  for (const suite of report.suites) lines.push('| ' + [suite.log, suite.crate, suite.kind, suite.target, suite.passed, suite.failed, suite.ignored].map(cell).join(' | ') + ' |');
  if (report.notices.length) {
    lines.push('', '## 完整性与缺测说明', '');
    for (const item of report.notices) lines.push(`- ${cell(item.severity)} — ${cell(item.log)}: ${cell(item.reason)}`);
  }
  fs.writeFileSync(path.join(directory, 'measurements.md'), lines.join('\n') + '\n');
  return report;
}

if (path.resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 3) throw Error('Usage: node scripts/baseline/report.mjs <output-directory>');
    const report = generateReport(process.argv[2]);
    console.log(`Recorded ${report.records.length} measurements across ${report.statistics.length} metrics; complete=${report.complete}.`);
    process.exitCode = report.complete ? 0 : 1;
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
