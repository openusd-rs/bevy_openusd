import assert from 'node:assert/strict';
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const directory = process.argv[2];
assert(directory, 'provide evidence directory');
const analyzer = new URL('./analyze.mjs', import.meta.url);
for (const name of ['budget', 'clock']) {
  const target = join(directory, `negative-${name}`);
  mkdirSync(target);
  for (const run of ['bird-0', 'cow-0', 'cow-1', 'bird-1']) {
    for (const suffix of ['jsonl', 'stderr', 'exit', 'time']) {
      copyFileSync(join(directory, `${run}.${suffix}`), join(target, `${run}.${suffix}`));
    }
  }
  const file = join(target, 'bird-0.jsonl');
  const rows = readFileSync(file, 'utf8').trim().split('\n').map(line => JSON.parse(line));
  if (name === 'budget') rows.find(row => row.type === 'warmup').retained_bytes = 8 * 1024 * 1024 + 1;
  else rows.find(row => row.type === 'sample').clock += 0.01;
  writeFileSync(file, rows.map(row => JSON.stringify(row)).join('\n') + '\n');
  const result = spawnSync(process.execPath, [analyzer.pathname, target, join(directory, 'trace-analysis.json')], { encoding: 'utf8' });
  writeFileSync(join(target, 'verification.log'), result.stdout + result.stderr);
  assert.equal(result.status, 1, 'negative control must fail');
  assert.match(result.stderr, name === 'budget' ? /source budget exceeded/ : /clock mismatch/);
  console.log(`${name} negative control rejected with expected error`);
}
