import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

assert([3, 4].includes(process.argv.length), 'provide the evidence directory and optional cow gain');
const cowGain = Number(process.argv[3] ?? 5);
assert([5, 10].includes(cowGain), 'cow gain must be 5 or 10 percent');
const source = path.resolve(process.argv[2]);
const analyzer = fileURLToPath(new URL('analyze.mjs', import.meta.url));
for (const kind of ['clock', 'digest', 'performance']) {
  const destination = path.join(source, `negative-${kind}`);
  fs.mkdirSync(destination);
  for (const name of fs.readdirSync(source)) {
    if (name === 'results.tsv' || /^(bird|cow)-\d-(before|after)\.(log|time)$/.test(name)) {
      fs.copyFileSync(path.join(source, name), path.join(destination, name));
    }
  }
  for (const name of fs.readdirSync(destination)) {
    if (!name.endsWith('-after.log')) continue;
    const file = path.join(destination, name);
    let text = fs.readFileSync(file, 'utf8');
    if (kind === 'clock') {
      text = text.replace(/^(tangent_kernel_sample .*\bclock=)\S+/m, (_, prefix) => prefix + '999');
    } else if (kind === 'digest') {
      text = text.replace(/^(tangent_kernel_sample .*\bdigest=)[0-9a-f]{64}/m, (_, prefix) => prefix + '0'.repeat(64));
    } else {
      text = text.replace(/^(tangent_kernel_(?:sample|warmup) .*\belapsed_ns=)(\d+)/gm,
        (_, prefix, value) => prefix + String(BigInt(value) * 2n));
      const times = [...text.matchAll(/^tangent_kernel_sample .*\belapsed_ns=(\d+)/gm)]
        .map(match => Number(match[1])).sort((a, b) => a - b);
      assert.equal(times.length, 100);
      text = text.replace(/^tangent_kernel_summary .*/m, line => line
        .replace(/median_ns=\d+/, `median_ns=${times[49]}`)
        .replace(/p95_ns=\d+/, `p95_ns=${times[94]}`)
        .replace(/max_ns=\d+/, `max_ns=${times[99]}`));
    }
    fs.writeFileSync(file, text);
  }
  const result = spawnSync(process.execPath, [analyzer, destination, String(cowGain)], { encoding: 'utf8' });
  const log = `${result.stdout ?? ''}${result.stderr ?? ''}`;
  fs.writeFileSync(path.join(source, `negative-${kind}.log`), log);
  assert.equal(result.status, 1, `${kind}: analyzer did not reject`);
  if (kind === 'performance') {
    const analysis = JSON.parse(fs.readFileSync(path.join(destination, 'analysis.json')));
    assert.equal(analysis.required_cow_gain_percent, cowGain);
    assert(analysis.failures.includes(`cow: ${cowGain}% kernel target missed`));
    assert(analysis.failures.includes('bird: paired median regression exceeds 5%'));
    assert(analysis.comparisons.every(row => row.paired.every(percent => percent > 5)));
  } else {
    assert.match(log, kind === 'clock' ? /clock mismatch/ : /output\/work differs/);
  }
  console.log(`tangent_arena_negative_control kind=${kind} rejected=true`);
}
