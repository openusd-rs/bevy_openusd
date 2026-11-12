import fs from 'node:fs';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const base = process.argv[2], dir = `${base}/perf`;
const analyzer = fileURLToPath(new URL('analyze.mjs', import.meta.url));
for (const kind of ['clock', 'budget', 'performance']) {
  const result = spawnSync('oslo', ['make', 'run-headless-benchmark', '--binary', process.execPath,
    `--args=${analyzer} ${dir} --negative=${kind}`], { encoding: 'utf8' });
  fs.writeFileSync(`${base}/negative-${kind}.log`, result.stdout + result.stderr);
  assert.equal(result.status, 1, `${kind} negative control`);
  assert.equal(result.signal, null);
  if (kind !== 'performance') {
    assert((result.stdout + result.stderr).includes(kind === 'clock' ? 'paired clock mismatch' : 'source budget'));
  } else {
    const negative = JSON.parse(fs.readFileSync(`${dir}/negative-performance.json`, 'utf8'));
    const comparisons = negative.comparisons.filter(c => c.phase !== 'identical');
    assert.equal(comparisons.length, 28);
    assert(comparisons.every(c => !c.passed));
  }
}
const data = JSON.parse(fs.readFileSync(`${dir}/analysis.json`, 'utf8'));
const summaries = [];
for (const phase of ['identical', 'bypass', 'cached']) for (const name of ['bird', 'cow', 'arm', 'morph', 'subsets']) {
  const values = data.comparisons.filter(c => c.phase === phase && c.name === name);
  if (!values.length) continue;
  const range = key => [Math.min(...values.map(c => c[key])), Math.max(...values.map(c => c[key]))];
  summaries.push({ phase, name, failed: values.filter(c => !c.passed).length,
    strata: values.length, medianRatio: range('medianRatio'), p95Ratio: range('p95Ratio') });
}
const hosts = fs.readdirSync(dir).filter(f => f.endsWith('-before.host.json'))
  .map(f => JSON.parse(fs.readFileSync(`${dir}/${f}`, 'utf8')));
const loads = hosts.map(h => Number(h.load.split(' ')[0]));
const summary = { processes: data.processes, updates: data.updates, warmups: data.warmups,
  loadRange: [Math.min(...loads), Math.max(...loads)], summaries,
  negativeControls: ['clock', 'budget', '28 slowed non-control strata'] };
fs.writeFileSync(`${base}/summary.json`, JSON.stringify(summary, null, 2) + '\n');
console.log(JSON.stringify(summary, null, 2));
