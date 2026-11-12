import fs from 'node:fs';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';

const dir = process.argv[2], negative = process.argv[3];
const read = name => fs.readFileSync(`${dir}/${name}`, 'utf8');
const json = name => JSON.parse(read(name));
const jsonl = name => read(name).trim().split('\n').map(JSON.parse);
const fields = line => Object.fromEntries([...line.matchAll(/(\w+)=([^\s]+)/g)].map(m => [m[1], m[2]]));
const quantile = (values, p) => [...values].sort((a, b) => a - b)[Math.ceil(values.length * p) - 1];
const hash = path => createHash('sha256').update(fs.readFileSync(path)).digest('hex');
const payload = ['mesh_entities', 'subset_entities', 'mesh_assets', 'vertices', 'unreferenced_vertices',
  'vertex_bytes', 'index_bytes', 'morph_bytes', 'image_bytes', 'gpu_morph_entities',
  'peak_mesh_assets', 'peak_material_assets', 'peak_image_assets', 'cached_meshes',
  'cached_mesh_payload_bytes', 'cached_tangent_payload_bytes', 'cached_assembly_payload_bytes'];
const matrix = json('matrix.json'), inputs = json('inputs.json'), complete = json('complete.json');
const environment = json('environment.json');
assert.deepEqual(complete, { processes: 48, updates: 48000, warmups: 192,
  poolThreads: complete.poolThreads, inputsVerified: true });
assert(Number(complete.poolThreads) > 1);
assert.equal(matrix.length, 24);
const expectedIds = [];
for (const phase of ['identical', 'bypass', 'cached']) for (const name of phase === 'bypass'
  ? ['bird', 'cow'] : ['bird', 'cow', 'arm', 'morph', 'subsets']) for (const reverse of [0, 1]) {
  expectedIds.push(`${phase}-${name}-${reverse}`);
}
assert.deepEqual(matrix.map(p => p.id).sort(), expectedIds.sort());
for (const [path, expected] of Object.entries(inputs)) assert.equal(hash(path), expected, path);
const statuses = jsonl('results.jsonl');
assert.equal(statuses.length, 48); assert.equal(new Set(statuses.map(s => s.id)).size, 48);
const runs = [], comparisons = [], failures = [], pids = new Set();

function parseRun(pair, side, rows) {
  const id = `${pair.id}-${side}`, lines = read(`${id}.log`).split('\n');
  const version = pair.phase !== 'identical' && side === 'right' ? 'after' : 'before';
  assert.deepEqual(statuses.find(s => s.id === id), { id, version, code: 0, signal: null, diagnostics: 0 });
  assert(!/(^|\s)(ERROR|WARN)(\s|$)|panicked at|Error:/m.test(lines.join('\n')), id);
  const one = prefix => {
    const found = lines.filter(l => l.startsWith(prefix)); assert.equal(found.length, 1, `${id}: ${prefix}`);
    return found[0];
  };
  const config = fields(one('seek_step_config '));
  const settings = fields(one('asset='));
  assert.equal(settings.asset, `${environment.cwd}/${environment.assets[pair.name]}`);
  assert.equal(settings.profile, 'release'); assert.equal(settings.renderer, 'headless');
  assert.equal(settings.deformation, 'gpu-prepared'); assert.equal(settings.samples, '1');
  assert.equal(settings.assets, 'all-retained-not-visibility-filtered');
  assert.equal(config.samples, '1000'); assert.equal(config.warmup, '4');
  assert.equal(config.pool_threads, complete.poolThreads);
  assert.equal(config.scope, 'full-headless-editor-update'); assert.equal(config.excludes, 'stdin-wait,stdout,gpu');
  const ready = lines.filter(l => l.startsWith('seek_step_ready ')); assert.equal(ready.length, 1004);
  ready.forEach((line, i) => assert.equal(Number(fields(line).iteration), i));
  const samples = lines.filter(l => /^seek(?:_warmup)?_sample /.test(l)).map(fields);
  assert.equal(samples.length, 1004);
  const bounds = fields(one('seek_timeline_start='));
  const start = Number(bounds.seek_timeline_start), end = Number(bounds.seek_timeline_end);
  assert(Number.isFinite(start) && Number.isFinite(end) && end > start);
  samples.forEach((sample, i) => {
    const row = rows[i];
    assert.equal(row.iteration, i); assert.equal(row.first, Number([1, 2].includes(i % 4) !== pair.reverse));
    assert.equal(Number(sample.index), i < 4 ? i : i - 4);
    assert.equal(row.clock, Number(sample.clock), 'paired clock mismatch');
    assert.equal(row[`${side}_ns`], Number(sample.update_ns));
    assert(Number.isSafeInteger(row[`${side}_ns`]) && row[`${side}_ns`] > 0);
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
  for (const [key, p] of [['seek_median_us', .5], ['seek_p95_us', .95], ['seek_max_us', 1]]) {
    assert(Math.abs(csv[key] - quantile(rows.slice(4).map(r => r[`${side}_ns`]), p) / 1000) <= .00051);
  }
  const work = {};
  work.image_buffers = fields(one('image_buffer_scope='));
  for (const prefix of ['mesh_preparation_profile', 'mesh_read_profile', 'gpu_skin_preparation_profile']) {
    work[prefix] = lines.filter(l => l.startsWith(`${prefix} phase=measured-seeks `)).map(line =>
      Object.fromEntries([...line.matchAll(/\b([a-z_]+)=(\d+)(?= |$)/g)]
        .filter(m => !/(_ms|_ns|_us)$/.test(m[1])).map(m => [m[1], Number(m[2])])));
  }
  const operationRows = lines.filter(l => l.startsWith('animation_access_profile phase=measured-seeks '));
  const builds = operationRows.filter(l => l.includes('operation=query-build '))
    .reduce((s, l) => s + Number(fields(l).calls), 0);
  const total = fields(one('animation_access_totals phase=measured-seeks '));
  assert.equal(total.dropped, '0'); assert.equal(total.saturated, 'false');
  assert.equal(Number(total.calls), operationRows.reduce((s, l) => s + Number(fields(l).calls), 0));
  let usage = null;
  const usageLines = lines.filter(l => l.startsWith('animation_query_cache '));
  if (version === 'before') assert.equal(usageLines.length, 0);
  else {
    const snapshots = ['after-seek-warmup', 'measured-seeks'].map(phase => {
      const found = usageLines.filter(l => fields(l).phase === phase); assert.equal(found.length, 1);
      const source = fields(found[0]); assert.equal(source.scope, 'live-stage-query-owner-cumulative');
      const numeric = Object.fromEntries(Object.entries(source).filter(([k]) => !['phase', 'scope'].includes(k))
        .map(([k, v]) => [k, Number(v)]));
      assert(Object.values(numeric).every(v => Number.isSafeInteger(v) && v >= 0));
      if (negative === '--negative=budget' && pair.id === 'cached-bird-0' && phase === 'measured-seeks') {
        numeric.source_bytes = numeric.max_source_bytes + 1;
      }
      assert.equal(numeric.owner, 1);
      assert(numeric.queries <= numeric.max_queries && numeric.metadata_bytes <= numeric.max_metadata_bytes, 'metadata budget');
      assert(numeric.sources <= numeric.max_sources && numeric.source_bytes <= numeric.max_source_bytes, 'source budget');
      return numeric;
    });
    const [warm, final] = snapshots;
    const delta = Object.fromEntries(['lookups', 'hits', 'admissions', 'refusals', 'clears'].map(k => [k, final[k] - warm[k]]));
    assert(Object.values(delta).every(v => v >= 0));
    assert.equal(delta.lookups, builds); assert.equal(delta.clears, 0);
    if (pair.phase === 'bypass') {
      for (const s of snapshots) for (const key of ['queries', 'metadata_bytes', 'sources', 'source_bytes', 'hits', 'admissions']) assert.equal(s[key], 0);
      assert.equal(delta.refusals, builds);
    } else if (['bird', 'cow'].includes(pair.name)) {
      const n = pair.name === 'bird' ? 1 : 2;
      assert.equal(builds, n * 1000); assert.equal(delta.hits, builds);
      assert.equal(delta.admissions, 0); assert.equal(delta.refusals, 0);
      for (const s of snapshots) {
        assert.equal(s.queries, n); assert.equal(s.sources, n * 3);
        assert(s.source_bytes > 0 && s.metadata_bytes > 0);
      }
      assert.equal(warm.source_bytes, final.source_bytes);
      assert.equal(warm.metadata_bytes, final.metadata_bytes);
    }
    usage = { warm, final, delta };
  }
  const identities = jsonl(`${id}.identity.jsonl`); assert.equal(identities.length, 2);
  assert.deepEqual(identities.map(v => v.label), ['ready-0', 'ready-1003']);
  const pid = Number(fields(one('owned_app_pid=')).owned_app_pid);
  assert(!pids.has(pid)); pids.add(pid);
  const base = dir.slice(0, -'/perf'.length);
  for (const item of identities) {
    assert.equal(item.pid, pid); assert.equal(item.sha256, inputs[`${base}/${version}`]);
    assert.equal(item.allowed, identities[0].allowed);
    assert.equal(item.stat.slice(item.stat.lastIndexOf(')') + 2).split(' ')[19],
      identities[0].stat.slice(identities[0].stat.lastIndexOf(')') + 2).split(' ')[19]);
  }
  return { id, version, csv, work, operationRows, usage };
}

for (const pair of matrix) {
  const rows = jsonl(`${pair.id}.jsonl`); assert.equal(rows.length, 1004);
  assert.equal(pair.id, `${pair.phase}-${pair.name}-${Number(pair.reverse)}`);
  if (negative === '--negative=clock' && pair.id === 'identical-bird-0') rows[4].clock += 1;
  const left = parseRun(pair, 'left', rows), right = parseRun(pair, 'right', rows);
  for (const key of payload) assert.equal(left.csv[key], right.csv[key], `${pair.id}: ${key}`);
  assert.deepEqual(left.work, right.work, `${pair.id}: work counts`);
  assert.deepEqual(left.operationRows, right.operationRows, `${pair.id}: animation operations`);
  runs.push(left, right);
  for (const first of [0, 1]) {
    const selected = rows.slice(4).filter(r => r.first === first); assert.equal(selected.length, 500);
    const scale = negative === '--negative=performance' && pair.phase !== 'identical' ? 2 : 1;
    const a = selected.map(r => r.left_ns), b = selected.map(r => r.right_ns * scale);
    const medianRatio = quantile(selected.map(r => scale * r.right_ns / r.left_ns), .5);
    const p95Ratio = quantile(b, .95) / quantile(a, .95);
    const target = pair.phase === 'cached' && ['bird', 'cow'].includes(pair.name) ? .99 : 1.05;
    const passed = pair.phase === 'identical'
      ? Math.abs(medianRatio - 1) <= .01 && Math.abs(p95Ratio - 1) <= .05
      : medianRatio <= target && p95Ratio <= 1.05;
    const comparison = { id: pair.id, phase: pair.phase, name: pair.name, first, pairs: 500,
      medianRatio, p95Ratio, leftMedianMs: quantile(a, .5) / 1e6, rightMedianMs: quantile(b, .5) / 1e6,
      leftP95Ms: quantile(a, .95) / 1e6, rightP95Ms: quantile(b, .95) / 1e6, passed };
    comparisons.push(comparison); if (!passed) failures.push(comparison);
  }
}
const result = { processes: 48, updates: 48000, warmups: 192, comparisons, failures, runs,
  identicalPassed: failures.every(f => f.phase !== 'identical'),
  bypassPassed: failures.every(f => f.phase !== 'bypass'),
  cachedPassed: failures.every(f => f.phase !== 'cached') };
fs.writeFileSync(`${dir}/${negative ? 'negative-performance' : 'analysis'}.json`, JSON.stringify(result, null, 2) + '\n');
for (const c of comparisons) console.log(JSON.stringify(c));
console.log(`VALIDATED processes=48 updates=48000 warmups=192 failures=${failures.length}`);
process.exitCode = failures.length ? 1 : 0;
