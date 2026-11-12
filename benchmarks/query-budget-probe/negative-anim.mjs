import assert from 'node:assert/strict';
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const directory = process.argv[2];
const analyzer = new URL('./analyze-anim.mjs', import.meta.url);
const summary = JSON.parse(readFileSync(join(directory, 'analysis.json'), 'utf8'));
for (const name of ['budget', 'clock', 'counts', 'performance']) {
  const target = join(directory, `negative-${name}`);
  mkdirSync(target);
  for (const run of summary.runs) {
    for (const suffix of ['jsonl', 'stderr', 'exit', 'time']) {
      copyFileSync(join(directory, `${run.name}.${suffix}`), join(target, `${run.name}.${suffix}`));
    }
  }
  const modified = name === 'performance' ? summary.runs.map(run => run.name) : [summary.runs[0].name];
  for (const run of modified) {
    const file = join(target, `${run}.jsonl`);
    const rows = readFileSync(file, 'utf8').trim().split('\n').map(JSON.parse);
    if (name === 'budget') rows[1].retained_bytes = rows[0].max_bytes + 1;
    else if (name === 'clock') rows.find(row => row.type === 'sample').clock += 0.01;
    else if (name === 'counts') rows[1].joint_counts[0] += 1;
    else for (const row of rows) if (row.type === 'sample' || row.type === 'warmup') row.query_ns = row.direct_ns * 2;
    writeFileSync(file, rows.map(row => JSON.stringify(row)).join('\n') + '\n');
  }
  const child = spawnSync(process.execPath, [analyzer.pathname, target, join(directory, 'trace-analysis.json'), summary.protocol], { encoding: 'utf8' });
  writeFileSync(join(target, 'verification.log'), child.stdout + child.stderr);
  assert.equal(child.status, 1, 'negative control must fail');
  if (name === 'performance') {
    const result = JSON.parse(readFileSync(join(target, 'analysis.json'), 'utf8'));
    assert.equal(result.violations.length, summary.runs.length * 2);
  } else {
    assert.match(child.stderr, { budget: /source budget exceeded/, clock: /clock mismatch/, counts: /joint counts changed/ }[name]);
  }
  console.log(`${name} negative control rejected`);
}
