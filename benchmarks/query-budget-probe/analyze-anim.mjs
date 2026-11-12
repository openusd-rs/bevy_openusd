import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [directory, traceFile, protocol = 'admit'] = process.argv.slice(2);
assert(directory && traceFile, 'provide results and trace');
assert(['admit', 'mixed', 'refusal'].includes(protocol));
const trace = JSON.parse(readFileSync(traceFile, 'utf8'));
const policies = { admit: [8 * 1024 * 1024, 8], 'small-sources': [64 * 1024, 8],
  'zero-bytes': [0, 8], 'one-byte': [1, 8], 'zero-entries': [8 * 1024 * 1024, 0] };
const cases = protocol !== 'refusal'
  ? ['bird-0', 'cow-0', 'cow-1', 'bird-1'].map(name => ({ name, asset: name.split('-')[0], reverse: name.endsWith('-1'), policy: protocol === 'admit' ? 'admit' : 'small-sources' }))
  : [false, true].flatMap(reverse => (reverse ? ['zero-entries', 'one-byte', 'zero-bytes'] : ['zero-bytes', 'one-byte', 'zero-entries'])
      .flatMap(policy => (reverse ? ['cow', 'bird'] : ['bird', 'cow']).map(asset => ({ name: `${asset}-${policy}-${Number(reverse)}`, asset, policy, reverse }))));
const read = (name, suffix) => readFileSync(join(directory, `${name}.${suffix}`), 'utf8');
const quantiles = values => {
  const sorted = [...values].sort((a, b) => a - b);
  const rank = p => sorted[Math.ceil(p * sorted.length) - 1];
  return { median_ns: rank(0.5), p95_ns: rank(0.95), max_ns: sorted.at(-1) };
};
const prior = new Map();
const violations = [];
const runs = cases.map(({ name, asset, reverse, policy }) => {
  assert.equal(read(name, 'exit').trim(), '0', 'process failed');
  assert.equal(read(name, 'stderr').trim(), '', 'application diagnostics');
  const resources = read(name, 'time').trim();
  assert.match(resources, /exit=0$/);
  const peakRss = Number(resources.match(/peak_rss_kb=(\d+)/)?.[1]);
  assert(Number.isSafeInteger(peakRss) && peakRss > 0);
  const rows = read(name, 'jsonl').trim().split('\n').map(JSON.parse);
  assert.equal(rows.length, 1006);
  const config = rows[0];
  const [maxBytes, maxEntries] = policies[policy];
  const reference = trace.runs.find(run => run.name === asset);
  assert(reference);
  const channels = reference.channels.map(channel => channel.path);
  const animations = [...new Set(channels.map(path => path.slice(0, path.lastIndexOf('.'))))].sort();
  assert.equal(config.type, 'config');
  assert.equal(config.scope, 'animation-local-matrices-not-editor-fps');
  assert.equal(config.samples, 1000);
  assert.equal(config.warmup, 4);
  assert.equal(config.reverse, reverse);
  assert.equal(config.policy, policy);
  assert.equal(config.max_bytes, maxBytes);
  assert.equal(config.max_entries, maxEntries);
  assert.deepEqual(config.channels, channels);
  assert.deepEqual(config.animations, animations);
  assert.equal(animations.length, asset === 'bird' ? 1 : 2);
  assert.equal(config.joint_orders.length, animations.length);
  assert(config.joint_orders.every(order => Array.isArray(order) && order.length > 0 && order.every(name => typeof name === 'string')));
  assert(Number.isFinite(config.start) && Number.isFinite(config.end) && config.start < config.end);
  assert(Array.isArray(config.layers) && config.layers.length > 0);
  const counts = config.joint_orders.map(order => order.length);
  const expectedEntries = protocol === 'admit' ? channels.length : protocol === 'mixed' ? animations.length : 0;
  const timed = rows.slice(1, -1);
  const charged = timed[0].retained_bytes;
  const signatures = [];
  const clocks = [];
  timed.forEach((row, iteration) => {
    const warm = iteration < 4;
    const fraction = warm ? [0, 0.5, 1, 0.5][iteration] : (iteration - 3) / 1000;
    assert.equal(row.type, warm ? 'warmup' : 'sample');
    assert.equal(row.index, warm ? iteration : iteration - 4);
    assert.equal(row.clock, config.start + (config.end - config.start) * fraction, 'clock mismatch');
    assert.equal(row.first, Number(([1, 2].includes(iteration % 4)) !== reverse));
    assert.deepEqual(row.joint_counts, counts, 'joint counts changed');
    assert.match(row.digest, /^[0-9a-f]{64}$/);
    for (const key of ['direct_ns', 'query_ns']) assert(Number.isSafeInteger(row[key]) && row[key] > 0);
    assert(Number.isSafeInteger(row.retained_bytes) && row.retained_bytes >= 0 && row.retained_bytes <= maxBytes, 'source budget exceeded');
    assert.equal(row.retained_entries, expectedEntries);
    assert.equal(row.retained_bytes, charged);
    if (expectedEntries === 0) assert.equal(row.retained_bytes, 0, 'source budget exceeded');
    else assert(row.retained_bytes > 0);
    signatures.push([row.type, row.index, row.clock, row.digest, row.joint_counts]);
    if (!warm) clocks.push(row.clock);
  });
  assert.equal(new Set(clocks).size, 1000);
  assert(clocks.every((clock, index) => index === 0 || clock > clocks[index - 1]));
  assert.deepEqual(rows.at(-1), { type: 'complete', samples: 1000, warmup: 4, animations: animations.length, retained_bytes: 0, retained_entries: 0 });
  const comparable = { signatures, channels, animations, orders: config.joint_orders, start: config.start, end: config.end, layers: [...config.layers].sort() };
  if (prior.has(asset)) assert.deepEqual(comparable, prior.get(asset), 'fresh-process answers changed');
  else prior.set(asset, comparable);
  const strata = [0, 1].map(first => {
    const samples = timed.filter(row => row.type === 'sample' && row.first === first);
    assert.equal(samples.length, 500);
    const direct = quantiles(samples.map(row => row.direct_ns));
    const query = quantiles(samples.map(row => row.query_ns));
    const change = (query.median_ns / direct.median_ns - 1) * 100;
    const bound = protocol === 'admit' ? -25 : 5;
    if (change > bound) violations.push(`${name} first=${first}: ${change.toFixed(4)}% exceeds ${bound}%`);
    return { first, samples: 500, direct, query, median_change_percent: change };
  });
  return { name, policy, animations: animations.length, joint_counts: counts, samples: 1000, warmups: 4,
    charged_source_bytes: charged, retained_entries: expectedEntries, peak_rss_kb: peakRss, strata,
    compared_animation_answers: 1004 * animations.length, compared_matrices: 1004 * counts.reduce((a, b) => a + b, 0),
    warmup_samples: timed.slice(0, 4) };
});
const result = { protocol, gate_passed: violations.length === 0, violations, runs,
  measured_updates: cases.length * 1000, warmups: cases.length * 4,
  compared_animation_answers: runs.reduce((sum, run) => sum + run.compared_animation_answers, 0),
  compared_matrices: runs.reduce((sum, run) => sum + run.compared_matrices, 0),
  scope: 'animation-local-matrix-batches; not skinning, editor, native FPS or a process-memory limit' };
writeFileSync(join(directory, 'analysis.json'), JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify(result, null, 2));
process.exitCode = result.gate_passed ? 0 : 1;
