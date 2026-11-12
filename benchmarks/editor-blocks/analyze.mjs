import fs from 'node:fs';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { names, blockSizes, blockPlan, fields, StepProtocol } from './protocol.mjs';

const dir = process.argv[2], negative = process.argv[3];
assert(dir && [undefined, '--negative=clock', '--negative=block', '--negative=payload', '--negative=performance'].includes(negative));
const read = name => fs.readFileSync(`${dir}/${name}`, 'utf8');
const json = name => JSON.parse(read(name));
const jsonl = name => read(name).trim().split('\n').map(JSON.parse);
const quantile = (values, p) => [...values].sort((a, b) => a - b)[Math.ceil(values.length * p) - 1];
const hash = path => createHash('sha256').update(fs.readFileSync(path)).digest('hex');
const payload = ['mesh_entities', 'subset_entities', 'mesh_assets', 'vertices', 'unreferenced_vertices',
  'vertex_bytes', 'index_bytes', 'morph_bytes', 'image_bytes', 'gpu_morph_entities',
  'peak_mesh_assets', 'peak_material_assets', 'peak_image_assets', 'cached_meshes',
  'cached_mesh_payload_bytes', 'cached_tangent_payload_bytes', 'cached_assembly_payload_bytes'];
const matrix = json('matrix.json'), inputs = json('inputs.json'), complete = json('complete.json');
const environment = json('environment.json');
const expected = [false, true].flatMap(reverse => (reverse ? [...names].reverse() : names)
  .map(name => ({ id: `identical-${name}-${Number(reverse)}`, name, reverse, size: blockSizes[name] })));
assert.deepEqual(matrix, expected);
assert.deepEqual(complete, { processes: 20, updates: 20000, warmups: 80,
  blockSizes, poolThreads: complete.poolThreads, inputsVerified: true });
assert(Number(complete.poolThreads) > 1);
assert.deepEqual(environment.blockSizes, blockSizes);
for (const [path, expected] of Object.entries(inputs)) assert.equal(hash(path), expected, path);
const base = dir.slice(0, -'/perf'.length);
assert.equal(inputs[`${base}/before`], '8297f07802f819c4866e8faa4dd63cd85f4ae5d07fbf689223427b9da08eced1');
assert.equal(hash(`${dir}/run`), inputs[`${base}/before`]);
const statuses = jsonl('results.jsonl');
assert.equal(statuses.length, 20); assert.equal(new Set(statuses.map(s => s.id)).size, 20);
const runs = [], comparisons = [], failures = [], pids = new Set();

function parseRun(pair, side, rows) {
  const id = `${pair.id}-${side}`, lines = read(`${id}.log`).split('\n');
  assert.deepEqual(statuses.find(s => s.id === id), { id, code: 0, signal: null, diagnostics: 0 });
  assert(!/(^|\s)(ERROR|WARN)(\s|$)|panicked at|Error:/m.test(lines.join('\n')), id);
  const one = prefix => {
    const found = lines.filter(l => l.startsWith(prefix)); assert.equal(found.length, 1, `${id}: ${prefix}`);
    return found[0];
  };
  const protocol = new StepProtocol();
  for (const line of lines) protocol.consume(line);
  assert.equal(protocol.ready, 1003); assert.equal(protocol.samples.length, 1004);
  const config = fields(one('seek_step_config ')), settings = fields(one('asset='));
  assert.equal(settings.asset, `${environment.cwd}/${environment.assets[pair.name]}`);
  assert.equal(settings.profile, 'release'); assert.equal(settings.renderer, 'headless');
  assert.equal(settings.deformation, 'gpu-prepared'); assert.equal(settings.samples, '1');
  assert.equal(settings.assets, 'all-retained-not-visibility-filtered');
  assert.equal(config.samples, '1000'); assert.equal(config.warmup, '4');
  assert.equal(config.pool_threads, complete.poolThreads);
  assert.equal(config.scope, 'full-headless-editor-update'); assert.equal(config.excludes, 'stdin-wait,stdout,gpu');
  const bounds = fields(one('seek_timeline_start='));
  const start = Number(bounds.seek_timeline_start), end = Number(bounds.seek_timeline_end);
  assert(Number.isFinite(start) && Number.isFinite(end) && end > start);
  const schedule = blockPlan(pair.name, pair.reverse).flatMap(b => Array.from({ length: b.count }, (_, offset) => ({ ...b, offset })));
  protocol.samples.forEach((sample, i) => {
    const row = rows[i], expected = schedule[i];
    assert.equal(row.iteration, i);
    assert.equal(row.block, expected.block, 'block schedule mismatch');
    assert.equal(row.offset, expected.offset, 'block schedule mismatch');
    assert.equal(row.first, expected.first, 'block schedule mismatch');
    assert.equal(row.clock, sample.clock, 'paired clock mismatch');
    assert.equal(row[`${side}_ns`], sample.ns);
    const fraction = i < 4 ? [0, .5, 1, .5][i] : (i - 3) / 1000;
    assert(Math.abs(row.clock - (start + (end - start) * fraction)) < 1e-9);
  });
  assert.equal(new Set(rows.slice(4).map(r => r.clock)).size, 1000);
  const summary = fields(one('seek_warmup_summary '));
  assert.equal(Number(summary.samples), 4);
  assert.equal(Number(summary.total_ns), rows.slice(0, 4).reduce((s, r) => s + r[`${side}_ns`], 0));
  assert.equal(Number(summary.max_ns), Math.max(...rows.slice(0, 4).map(r => r[`${side}_ns`])));
  const header = one('sample,open_ms,').split(',');
  const rawCsv = lines.filter(l => /^0,[\d.]+,/.test(l)); assert.equal(rawCsv.length, 1);
  const values = rawCsv[0].split(','); assert.equal(values.length, header.length);
  const csv = Object.fromEntries(header.map((k, i) => [k, values[i] === 'NA' ? null : Number(values[i])]));
  assert(Object.values(csv).every(v => v === null || Number.isFinite(v)));
  for (const key of payload) assert(Number.isSafeInteger(csv[key]) && csv[key] >= 0, key);
  for (const [key, p] of [['seek_median_us', .5], ['seek_p95_us', .95], ['seek_max_us', 1]]) {
    assert(Math.abs(csv[key] - quantile(rows.slice(4).map(r => r[`${side}_ns`]), p) / 1000) <= .00051);
  }
  const work = { image_buffers: fields(one('image_buffer_scope=')) };
  for (const prefix of ['mesh_preparation_profile', 'mesh_read_profile', 'gpu_skin_preparation_profile']) {
    work[prefix] = lines.filter(l => l.startsWith(`${prefix} phase=measured-seeks `)).map(line =>
      Object.fromEntries([...line.matchAll(/\b([a-z_]+)=(\d+)(?= |$)/g)]
        .filter(m => !/(_ms|_ns|_us)$/.test(m[1])).map(m => [m[1], Number(m[2])])));
  }
  if (['bird', 'cow'].includes(pair.name)) assert(work.mesh_preparation_profile.length > 0);
  const operationRows = lines.filter(l => l.startsWith('animation_access_profile phase=measured-seeks '));
  const total = fields(one('animation_access_totals phase=measured-seeks '));
  assert.equal(total.dropped, '0'); assert.equal(total.saturated, 'false');
  assert.equal(Number(total.calls), operationRows.reduce((s, l) => s + Number(fields(l).calls), 0));
  assert.equal(lines.filter(l => l.startsWith('animation_query_cache ')).length, 0);
  const identities = jsonl(`${id}.identity.jsonl`); assert.equal(identities.length, 4);
  assert.deepEqual(identities.map(v => v.label), ['ready-0', 'ready-4', 'ready-504', 'ready-last-block']);
  assert.deepEqual(identities.map(v => v.ready), [0, 4, 504, 1004 - pair.size]);
  const pid = Number(fields(one('owned_app_pid=')).owned_app_pid);
  assert(!pids.has(pid)); pids.add(pid);
  const allowed = environment.controllerStatus.match(/^Cpus_allowed_list:\s*(.+)$/m)[1];
  for (const item of identities) {
    assert.equal(item.pid, pid); assert.equal(item.sha256, inputs[`${base}/before`]);
    assert.equal(item.allowed, allowed);
    assert.equal(item.stat.slice(item.stat.lastIndexOf(')') + 2).split(' ')[19],
      identities[0].stat.slice(identities[0].stat.lastIndexOf(')') + 2).split(' ')[19]);
  }
  return { id, csv, work, operationRows };
}

for (const pair of matrix) {
  const rows = jsonl(`${pair.id}.jsonl`); assert.equal(rows.length, 1004);
  if (negative === '--negative=clock' && pair.id === 'identical-bird-0') rows[4].clock += 1;
  if (negative === '--negative=block' && pair.id === 'identical-bird-0') rows[4].block += 1;
  const left = parseRun(pair, 'left', rows), right = parseRun(pair, 'right', rows);
  const blocks = jsonl(`${pair.id}.blocks.jsonl`), plan = blockPlan(pair.name, pair.reverse);
  assert.equal(blocks.length, plan.length);
  let previousEnd = 0;
  blocks.forEach((block, i) => {
    const { intervals, ...metadata } = block;
    assert.deepEqual(metadata, plan[i]); assert.equal(intervals.length, 2);
    assert.deepEqual(intervals.map(v => v.side), block.first ? ['right', 'left'] : ['left', 'right']);
    for (const interval of intervals) {
      assert(['start_ns', 'end_ns', 'update_ns'].every(k => Number.isSafeInteger(interval[k]) && interval[k] > 0));
      assert(interval.start_ns >= previousEnd && interval.end_ns > interval.start_ns, 'block intervals overlap');
      const elapsed = rows.slice(block.start, block.start + block.count).reduce((sum, r) => sum + r[`${interval.side}_ns`], 0);
      assert.equal(interval.update_ns, elapsed);
      assert(interval.end_ns - interval.start_ns >= elapsed);
      previousEnd = interval.end_ns;
    }
  });
  if (negative === '--negative=payload' && pair.id === 'identical-bird-0') right.csv.mesh_assets += 1;
  for (const key of payload) assert.equal(left.csv[key], right.csv[key], `${pair.id}: payload mismatch ${key}`);
  assert.deepEqual(left.work, right.work, `${pair.id}: work counts`);
  assert.deepEqual(left.operationRows, right.operationRows, `${pair.id}: animation operations`);
  runs.push(left, right);
  for (const first of [0, 1]) {
    const selected = rows.slice(4).filter(r => r.first === first); assert.equal(selected.length, 500);
    const scale = negative === '--negative=performance' ? 2 : 1;
    const a = selected.map(r => r.left_ns), b = selected.map(r => r.right_ns * scale);
    const medianRatio = quantile(selected.map(r => scale * r.right_ns / r.left_ns), .5);
    const p95Ratio = quantile(b, .95) / quantile(a, .95);
    const passed = Math.abs(medianRatio - 1) <= .01 && Math.abs(p95Ratio - 1) <= .05;
    const comparison = { id: pair.id, name: pair.name, first, pairs: 500,
      medianRatio, p95Ratio, leftMedianMs: quantile(a, .5) / 1e6, rightMedianMs: quantile(b, .5) / 1e6,
      leftP95Ms: quantile(a, .95) / 1e6, rightP95Ms: quantile(b, .95) / 1e6, passed };
    comparisons.push(comparison); if (!passed) failures.push(comparison);
  }
}
const result = { processes: 20, updates: 20000, warmups: 80, blockSizes,
  comparisons, failures, runs, controlsPassed: failures.length === 0 };
fs.writeFileSync(`${dir}/${negative ? 'negative-performance' : 'analysis'}.json`, JSON.stringify(result, null, 2) + '\n');
for (const c of comparisons) console.log(JSON.stringify(c));
console.log(`VALIDATED processes=20 updates=20000 warmups=80 failures=${failures.length}`);
process.exitCode = failures.length ? 1 : 0;
