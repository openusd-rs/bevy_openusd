import fs from 'node:fs';
import assert from 'node:assert/strict';
import { blockPlan } from '../editor-blocks/protocol.mjs';
import { parseThread, difference } from './scheduler.mjs';

const dir = process.argv[2], negative = process.argv[3];
assert(dir && [undefined, '--negative=counter', '--negative=identity'].includes(negative));
const json = file => JSON.parse(fs.readFileSync(`${dir}/${file}`, 'utf8'));
const jsonl = file => fs.readFileSync(`${dir}/${file}`, 'utf8').trim().split('\n').map(JSON.parse);
const environment = json('environment.json'), functional = json('analysis.json'), matrix = json('matrix.json');
assert(environment.scope.startsWith('scheduler-diagnostic-only;'));
assert.equal(functional.processes, 20); assert.equal(functional.updates, 20000); assert.equal(functional.warmups, 80);
assert.equal(functional.runs.length, 20);
const cpus = value => value.split(',').flatMap(range => {
  assert(/^\d+(?:-\d+)?$/.test(range));
  const [a, b = a] = range.split('-').map(Number);
  assert(b >= a && b - a < 1024); return Array.from({ length: b - a + 1 }, (_, i) => i + a);
});
const core = cpus(environment.cpuMaps.core), atom = cpus(environment.cpuMaps.atom);
assert(core.length && atom.length && core.every(cpu => !atom.includes(cpu)));
const allowed = environment.controllerStatus.match(/^Cpus_allowed_list:\s*(.+)$/m)[1];
const type = cpu => core.includes(cpu) ? 'core' : atom.includes(cpu) ? 'atom' : assert.fail(`unknown CPU ${cpu}`);
const median = values => [...values].sort((a, b) => a - b)[Math.ceil(values.length / 2) - 1];
const records = [], pairs = [], threadCounts = new Set();
let warmups = 0, terminal = 0, snapshots = 0;

function validateSnapshot(snapshot, id, ready) {
  assert.equal(snapshot.ready, ready); assert.equal(snapshot.schedstats, environment.schedstats);
  assert(snapshot.start_ns > 0 && snapshot.end_ns >= snapshot.start_ns);
  assert([snapshot.start_ns, snapshot.end_ns].every(Number.isSafeInteger));
  const pid = jsonl(`${id}.identity.jsonl`)[0].pid; assert.equal(snapshot.pid, pid);
  const tids = snapshot.threads.map(t => t.tid);
  assert.deepEqual(tids, [...new Set(tids)].sort((a, b) => a - b)); assert(tids.includes(pid));
  threadCounts.add(tids.length);
  for (const item of snapshot.threads) {
    const { raw, ...parsed } = item;
    assert.deepEqual(parsed, parseThread(item.tid, raw), 'snapshot parsed counters mismatch');
    assert.equal(parsed.allowed, allowed); type(parsed.cpu);
  }
  assert.equal(snapshot.threads.find(t => t.tid === pid).state, 'S'); snapshots++;
}

for (const pair of matrix) {
  const schedule = blockPlan(pair.name, pair.reverse), updates = jsonl(`${pair.id}.jsonl`), sides = {};
  for (const side of ['left', 'right']) {
    const id = `${pair.id}-${side}`, entries = jsonl(`${id}.scheduler.jsonl`);
    assert.equal(entries.length, schedule.length);
    const values = [];
    let previous;
    for (let i = 0; i < entries.length; i++) {
      const entry = entries[i], { before, after, delta, ...block } = entry;
      assert.deepEqual(block, schedule[i]);
      validateSnapshot(before, id, block.start);
      if (previous) difference(previous, before);
      if (i === entries.length - 1) {
        assert.equal(after, null); assert.equal(delta, null); terminal++;
        continue;
      }
      validateSnapshot(after, id, block.start + block.count);
      if (negative === '--negative=identity' && pair.id === 'identical-bird-0' && side === 'left' && i === 1) after.pid++;
      if (negative === '--negative=counter' && pair.id === 'identical-bird-0' && side === 'left' && i === 1) delta.main.cpu_ns++;
      assert.deepEqual(delta, difference(before, after), 'scheduler delta mismatch');
      previous = after;
      if (block.block === -1) { warmups++; continue; }
      const slice = updates.slice(block.start, block.start + block.count);
      const update_ns = slice.reduce((sum, sample) => sum + sample[`${side}_ns`], 0);
      assert(update_ns > 0 && update_ns <= delta.window_ns);
      const record = { id, pair: pair.id, name: pair.name, side, block: block.block, count: block.count,
        first: block.first, mainClass: delta.main.migrations === 0 ? type(delta.main.from_cpu) : 'migrated',
        update_ns, ...delta };
      values.push(record); records.push(record);
    }
    sides[side] = values;
  }
  assert.equal(sides.left.length, sides.right.length);
  for (let i = 0; i < sides.left.length; i++) {
    const left = sides.left[i], right = sides.right[i]; assert.equal(left.block, right.block);
    pairs.push({ id: pair.id, name: pair.name, block: left.block, first: left.first,
      leftClass: left.mainClass, rightClass: right.mainClass,
      updateRatio: right.update_ns / left.update_ns, mainCpuRatio: right.main.cpu_ns / left.main.cpu_ns,
      windowRatio: right.window_ns / left.window_ns,
      leftMainCpuFraction: left.main.cpu_ns / left.window_ns,
      rightMainCpuFraction: right.main.cpu_ns / right.window_ns });
  }
}
assert.equal(records.length, 428); assert.equal(pairs.length, 214);
assert.equal(warmups, 20); assert.equal(terminal, 20); assert.equal(snapshots, 916);
const groups = [...new Set(records.map(r => `${r.name}/${r.mainClass}`))].map(key => {
  const selected = records.filter(r => `${r.name}/${r.mainClass}` === key);
  const sum = fn => selected.reduce((s, r) => s + fn(r), 0);
  return { key, blocks: selected.length, updates: sum(r => r.count),
    medianBlockMeanUpdateUs: median(selected.map(r => r.update_ns / r.count / 1000)),
    medianBlockMeanMainCpuUs: median(selected.map(r => r.main.cpu_ns / r.count / 1000)),
    medianBlockMeanWorkerCpuUs: median(selected.map(r => r.worker_cpu_ns / r.count / 1000)),
    mainCpuFraction: sum(r => r.main.cpu_ns) / sum(r => r.window_ns),
    captureFraction: sum(r => r.capture_ns) / sum(r => r.window_ns),
    migrations: sum(r => r.main.migrations), voluntary: sum(r => r.main.voluntary),
    involuntary: sum(r => r.main.involuntary) };
});
const pairGroups = [...new Set(pairs.map(p => `${p.name}/${p.leftClass}/${p.rightClass}`))].map(key => {
  const selected = pairs.filter(p => `${p.name}/${p.leftClass}/${p.rightClass}` === key);
  return { key, blocks: selected.length, medianUpdateRatio: median(selected.map(p => p.updateRatio)),
    medianMainCpuRatio: median(selected.map(p => p.mainCpuRatio)),
    minUpdateRatio: Math.min(...selected.map(p => p.updateRatio)), maxUpdateRatio: Math.max(...selected.map(p => p.updateRatio)) };
});
const summary = { diagnosticOnly: true, processes: 20, measuredUpdates: 20000, warmups: 80,
  schedulerMeasuredBlocks: records.length, schedulerWarmupBlocks: warmups, terminalBlocksUnavailable: terminal,
  snapshots, threadCounts: [...threadCounts], schedstats: environment.schedstats,
  runqueueWaitAvailable: environment.schedstats === '1', cpuMaps: environment.cpuMaps,
  timingControlFailuresNotAcceptance: functional.failures.length, groups, pairGroups };
fs.writeFileSync(`${dir}/scheduler-analysis.json`, JSON.stringify({ summary, records, pairs }, null, 2) + '\n');
console.log(JSON.stringify(summary, null, 2));
