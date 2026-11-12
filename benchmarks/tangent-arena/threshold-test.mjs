import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

assert.equal(process.argv.length, 4, 'provide source evidence and a new fixture directory');
const source = path.resolve(process.argv[2]), output = path.resolve(process.argv[3]);
fs.mkdirSync(output);
fs.writeFileSync(path.join(output, 'SYNTHETIC.txt'), 'Synthetic 7.5% improvement: analyzer test, not a performance measurement.\n');
for (const name of fs.readdirSync(source)) {
  if (name === 'results.tsv' || /^(bird|cow)-\d-(before|after)\.(log|time)$/.test(name)) {
    fs.copyFileSync(path.join(source, name), path.join(output, name));
  }
}
for (const name of ['bird', 'cow']) {
  for (const [before, after] of [[1, 2], [4, 3]]) {
    let log = fs.readFileSync(path.join(source, `${name}-${before}-before.log`), 'utf8');
    log = log.replace(/^(tangent_kernel_(?:sample|warmup) .*\belapsed_ns=)(\d+)/gm,
      (_, prefix, value) => prefix + String(BigInt(value) * 37n / 40n));
    const times = [...log.matchAll(/^tangent_kernel_sample .*\belapsed_ns=(\d+)/gm)]
      .map(match => Number(match[1])).sort((a, b) => a - b);
    assert.equal(times.length, 100);
    log = log.replace(/^tangent_kernel_summary .*/m, line => line
      .replace(/median_ns=\d+/, `median_ns=${times[49]}`)
      .replace(/p95_ns=\d+/, `p95_ns=${times[94]}`)
      .replace(/max_ns=\d+/, `max_ns=${times[99]}`));
    fs.writeFileSync(path.join(output, `${name}-${after}-after.log`), log);
    fs.copyFileSync(path.join(source, `${name}-${before}-before.time`),
      path.join(output, `${name}-${after}-after.time`));
  }
}
const analyzer = fileURLToPath(new URL('analyze.mjs', import.meta.url));
for (const gain of [5, 10]) {
  const result = spawnSync(process.execPath, [analyzer, output, String(gain)], { encoding: 'utf8' });
  fs.writeFileSync(path.join(output, `threshold-${gain}.log`), `${result.stdout ?? ''}${result.stderr ?? ''}`);
  assert.equal(result.status, gain === 5 ? 0 : 1);
  const analysis = JSON.parse(fs.readFileSync(path.join(output, 'analysis.json')));
  assert.equal(analysis.required_cow_gain_percent, gain);
  assert.deepEqual(analysis.failures, gain === 5 ? [] : ['cow: 10% kernel target missed']);
  fs.renameSync(path.join(output, 'analysis.json'), path.join(output, `analysis-${gain}.json`));
}
console.log('tangent_threshold_control synthetic_gain=7.5 accepts=5 rejects=10');
