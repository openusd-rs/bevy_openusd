import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [directory, traceFile] = process.argv.slice(2);
assert(directory && traceFile, 'usage: analyze.mjs RESULTS TRACE_ANALYSIS_JSON');
const trace = JSON.parse(readFileSync(traceFile, 'utf8'));
const maxBytes = 8 * 1024 * 1024;
const names = ['bird-0', 'cow-0', 'cow-1', 'bird-1'];
const files = (name, suffix) => readFileSync(join(directory, `${name}.${suffix}`), 'utf8');
const quantiles = values => {
  const ordered = [...values].sort((a, b) => a - b);
  const rank = p => ordered[Math.ceil(p * ordered.length) - 1];
  return { median_ns: rank(0.5), p95_ns: rank(0.95), max_ns: ordered.at(-1) };
};
const prior = new Map();
const violations = [];
const runs = names.map(name => {
  const [asset, reverseText] = name.split('-');
  const reverse = reverseText === '1';
  const reference = trace.runs.find(run => run.name === asset);
  assert(reference, `missing trace for ${asset}`);
  assert.equal(files(name, 'exit').trim(), '0', `${name}: process failed`);
  assert.equal(files(name, 'stderr').trim(), '', `${name}: application diagnostics`);
  const resource = files(name, 'time').trim();
  assert.match(resource, /exit=0$/);
  const peakRss = Number(resource.match(/peak_rss_kb=(\d+)/)?.[1]);
  assert(Number.isSafeInteger(peakRss) && peakRss > 0, 'missing peak RSS');
  const rows = files(name, 'jsonl').trim().split('\n').map(line => JSON.parse(line));
  const config = rows[0];
  const expected = reference.channels.map(channel => channel.path);
  assert.equal(config.type, 'config');
  assert.equal(config.samples, 1000);
  assert.equal(config.warmup, 4);
  assert.equal(config.reverse, reverse);
  assert.equal(config.max_bytes, maxBytes);
  assert.equal(config.max_entries, 8);
  assert.deepEqual(config.paths, expected, 'channels differ from observed projection');
  assert(expected.length === (asset === 'bird' ? 3 : 6));
  assert(Number.isFinite(config.start) && Number.isFinite(config.end) && config.start < config.end);
  assert(Array.isArray(config.layers) && config.layers.length > 0);
  const inventory = rows.slice(1, 1 + expected.length);
  assert.deepEqual(inventory.map(row => row.path), expected);
  inventory.forEach((row, index) => {
    const original = reference.channels[index];
    assert.equal(row.type, 'inventory');
    assert.equal(row.samples, Number(original.samples));
    assert.equal(row.numeric_bytes, Number(original.numeric_payload_bytes));
    assert.equal(row.default_bytes, Number(original.default_numeric_bytes));
  });
  const timed = rows.slice(1 + expected.length, -1);
  assert.equal(timed.length, 1004);
  const charged = timed[0].retained_bytes;
  assert(charged > reference.numeric_payload_bytes, 'missing source storage overhead');
  const signatures = [];
  const clocks = [];
  timed.forEach((row, iteration) => {
    const warm = iteration < 4;
    const fraction = warm ? [0, 0.5, 1, 0.5][iteration] : (iteration - 3) / 1000;
    const expectedClock = config.start + (config.end - config.start) * fraction;
    assert.equal(row.type, warm ? 'warmup' : 'sample');
    assert.equal(row.index, warm ? iteration : iteration - 4);
    assert.equal(row.first, Number(([1, 2].includes(iteration % 4)) !== reverse));
    assert.equal(row.clock, expectedClock, 'clock mismatch');
    for (const key of ['direct_ns', 'query_ns']) {
      assert(Number.isSafeInteger(row[key]) && row[key] > 0, `invalid ${key}`);
    }
    assert.match(row.digest, /^[0-9a-f]{64}$/);
    assert(Number.isSafeInteger(row.retained_bytes) && row.retained_bytes > 0 && row.retained_bytes <= maxBytes, 'source budget exceeded');
    assert.equal(row.retained_bytes, charged, 'unchanged source charge drifted');
    assert.equal(row.retained_entries, expected.length);
    signatures.push([row.type, row.index, row.clock, row.digest]);
    if (!warm) clocks.push(row.clock);
  });
  assert.equal(new Set(clocks).size, 1000, 'non-unique measured clocks');
  assert(clocks.every((clock, index) => index === 0 || clock > clocks[index - 1]));
  assert.deepEqual(rows.at(-1), {
    type: 'complete', samples: 1000, warmup: 4, attributes: expected.length,
    retained_bytes: 0, retained_entries: 0,
  });
  const comparable = { signatures, inventory, paths: expected, start: config.start, end: config.end,
    layers: [...config.layers].sort(), charged };
  if (prior.has(asset)) assert.deepEqual(comparable, prior.get(asset), 'fresh-process answers or inventory differ');
  else prior.set(asset, comparable);
  const strata = [0, 1].map(first => {
    const samples = timed.filter(row => row.type === 'sample' && row.first === first);
    assert.equal(samples.length, 500);
    const direct = quantiles(samples.map(row => row.direct_ns));
    const query = quantiles(samples.map(row => row.query_ns));
    const change = (query.median_ns / direct.median_ns - 1) * 100;
    if (change > -25) violations.push(`${name} first=${first}: median change ${change.toFixed(4)}% misses -25%`);
    return { first, samples: samples.length, direct, query, median_change_percent: change };
  });
  return { name, attributes: expected.length, samples: 1000, warmup_updates: 4,
    numeric_payload_bytes: reference.numeric_payload_bytes, charged_source_bytes: charged,
    retained_entries: expected.length, peak_rss_kb: peakRss, strata,
    warmup_samples: timed.slice(0, 4).map(row => ({ first: row.first, clock: row.clock,
      direct_ns: row.direct_ns, query_ns: row.query_ns })) };
});
const result = { gate_passed: violations.length === 0, violations, runs,
  measured_updates: 4000, warmup_updates: 16,
  compared_attribute_answers: runs.reduce((sum, run) => sum + (1000 + 4) * run.attributes, 0),
  scope: 'used-animation-attribute-batches; not full editor, native FPS or a process-memory limit' };
writeFileSync(join(directory, 'analysis.json'), JSON.stringify(result, null, 2) + '\n');
process.stdout.write(JSON.stringify(result, null, 2) + '\n');
process.exitCode = result.gate_passed ? 0 : 1;
