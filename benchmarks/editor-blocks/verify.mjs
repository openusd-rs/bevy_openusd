import fs from 'node:fs';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { names } from './protocol.mjs';

const base = process.argv[2], dir = `${base}/perf`;
const analyzer = fileURLToPath(new URL('analyze.mjs', import.meta.url));
for (const [kind, diagnostic] of [['clock', 'paired clock mismatch'], ['block', 'block schedule mismatch'],
  ['payload', 'payload mismatch'], ['performance', null]]) {
  const result = spawnSync('oslo', ['make', 'run-headless-benchmark', '--binary', process.execPath,
    `--args=${analyzer} ${dir} --negative=${kind}`], { encoding: 'utf8' });
  fs.writeFileSync(`${base}/negative-${kind}.log`, result.stdout + result.stderr);
  assert.equal(result.status, 1, `${kind} negative control`); assert.equal(result.signal, null);
  if (diagnostic) assert((result.stdout + result.stderr).includes(diagnostic));
  else {
    const negative = JSON.parse(fs.readFileSync(`${dir}/negative-performance.json`, 'utf8'));
    assert.equal(negative.comparisons.length, 20);
    assert(negative.comparisons.every(c => !c.passed));
  }
}
const data = JSON.parse(fs.readFileSync(`${dir}/analysis.json`, 'utf8'));
const summaries = names.map(name => {
  const values = data.comparisons.filter(c => c.name === name);
  assert.equal(values.length, 4);
  const range = key => [Math.min(...values.map(c => c[key])), Math.max(...values.map(c => c[key]))];
  return { name, failed: values.filter(c => !c.passed).length,
    strata: values.length, medianRatio: range('medianRatio'), p95Ratio: range('p95Ratio') };
});
const loads = fs.readdirSync(dir).filter(f => f.endsWith('.host.json'))
  .map(f => Number(JSON.parse(fs.readFileSync(`${dir}/${f}`, 'utf8')).load.split(' ')[0]));
const summary = { processes: 20, updates: 20000, warmups: 80, controlsPassed: data.controlsPassed,
  loadRange: [Math.min(...loads), Math.max(...loads)], summaries,
  negativeControls: ['clock', 'block schedule', 'payload', '20 slowed strata'] };
fs.writeFileSync(`${base}/summary.json`, JSON.stringify(summary, null, 2) + '\n');
console.log(JSON.stringify(summary, null, 2));
