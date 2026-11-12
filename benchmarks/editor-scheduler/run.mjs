import fs from 'node:fs';
import os from 'node:os';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import readline from 'node:readline';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { names, blockSizes, blockPlan, StepProtocol } from '../editor-blocks/protocol.mjs';
import { snapshot, difference } from './scheduler.mjs';

assert.equal(process.argv.length, 3, 'provide the evidence directory');
const base = process.argv[2], cwd = process.cwd(), dir = `${base}/perf`;
assert(/^\/[\w./-]+$/.test(base) && /^\/[\w./-]+$/.test(cwd));
fs.mkdirSync(dir);
const launcher = fileURLToPath(new URL('../query-editor/launch.sh', import.meta.url));
const hash = path => createHash('sha256').update(fs.readFileSync(path)).digest('hex');
const assets = {
  bird: 'assets/external/hummingbird.usdz', cow: 'assets/external/cows/cow_eating.usda',
  arm: 'assets/external/universal_robots/ur10_pick.usda',
  morph: 'assets/skel_morph_blended_animated.usda', subsets: 'assets/material_subsets.usda',
};
const matrix = [false, true].flatMap(reverse => (reverse ? [...names].reverse() : names)
  .map(name => ({ id: `identical-${name}-${Number(reverse)}`, name, reverse, size: blockSizes[name] })));
const inputs = Object.fromEntries([
  `${base}/before`, launcher, fileURLToPath(import.meta.url),
  fileURLToPath(new URL('../editor-blocks/protocol.mjs', import.meta.url)),
  fileURLToPath(new URL('scheduler.mjs', import.meta.url)), 'Cargo.lock', ...Object.values(assets),
].map(file => [file, hash(file)]));
assert.equal(inputs[`${base}/before`], '8297f07802f819c4866e8faa4dd63cd85f4ae5d07fbf689223427b9da08eced1');
fs.copyFileSync(`${base}/before`, `${dir}/run`);
fs.chmodSync(`${dir}/run`, 0o755);
const cpuMaps = Object.fromEntries(['core', 'atom'].map(type =>
  [type, fs.readFileSync(`/sys/bus/event_source/devices/cpu_${type}/cpus`, 'utf8').trim()]));
const schedstats = fs.readFileSync('/proc/sys/kernel/sched_schedstats', 'utf8').trim();
const write = (name, value) => fs.writeFileSync(`${dir}/${name}`, JSON.stringify(value, null, 2) + '\n');
write('matrix.json', matrix); write('inputs.json', inputs);
write('environment.json', {
  date: new Date().toISOString(), cwd, assets, blockSizes, cpuMaps, schedstats, kernel: os.release(), cpu: os.cpus()[0].model,
  controllerStatus: fs.readFileSync('/proc/self/status', 'utf8'),
  scope: 'scheduler-diagnostic-only; no-performance-acceptance; default-affinity; no-owned-builds; all-updates-retained',
});
fs.writeFileSync(`${dir}/results.jsonl`, '');
const active = new Set();
function pressure(label) {
  write(`${label}.host.json`, {
    date: new Date().toISOString(), load: fs.readFileSync('/proc/loadavg', 'utf8'),
    pressure: Object.fromEntries(['cpu', 'memory', 'io'].map(k => [k, fs.readFileSync(`/proc/pressure/${k}`, 'utf8')])),
    processes: spawnSync('ps', ['-eo', 'pid,ppid,comm,args'], { encoding: 'utf8' }).stdout,
  });
}
function identity(lane, label) {
  const proc = `/proc/${lane.protocol.pid}`, status = fs.readFileSync(`${proc}/status`, 'utf8');
  const value = {
    pid: lane.protocol.pid, label, ready: lane.protocol.ready, sha256: hash(`${proc}/exe`),
    stat: fs.readFileSync(`${proc}/stat`, 'utf8'),
    allowed: status.match(/^Cpus_allowed_list:\s*(.+)$/m)[1],
  };
  assert.equal(value.sha256, inputs[`${base}/before`]);
  assert.equal(value.allowed, fs.readFileSync('/proc/self/status', 'utf8').match(/^Cpus_allowed_list:\s*(.+)$/m)[1]);
  fs.appendFileSync(`${dir}/${lane.id}.identity.jsonl`, JSON.stringify(value) + '\n');
}
async function stopOwned() {
  const remaining = [...active];
  for (const child of remaining) { try { process.kill(-child.pid, 'SIGTERM'); } catch {} }
  await Promise.all(remaining.map(child => child.closed));
}
for (const [signal, code] of [['SIGINT', 130], ['SIGTERM', 143]]) {
  process.once(signal, async () => { await stopOwned(); process.exit(code); });
}
async function openLane(pair, side) {
  const id = `${pair.id}-${side}`, protocol = new StepProtocol();
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('USD_')));
  Object.assign(env, {
    USD_BENCH_STEPPED: '1', USD_PROFILE_ROUTES: '1', USD_PROFILE_IMAGES: '1',
    USD_PROFILE_ANIMATION_READS: '1',
  });
  const stdout = fs.openSync(`${dir}/${id}.stdout`, 'wx'), stderr = fs.openSync(`${dir}/${id}.stderr`, 'wx');
  const child = spawn('oslo', ['make', 'run-headless-benchmark', '--binary', '/run/current-system/sw/bin/bash',
    `--args=${launcher} ${dir}/run ${cwd}/${assets[pair.name]} 1 gpu-prepared seek-timeline`],
  { env, detached: true, stdio: ['pipe', 'pipe', 'pipe'] });
  active.add(child);
  let ended = false, status, spawnError;
  const lines = [], pending = [];
  child.on('error', e => { spawnError = e.message; });
  child.stdin.on('error', e => { spawnError = e.message; });
  const closed = new Promise(resolve => child.on('close', (code, signal) => {
    ended = true; status = { code, signal, spawnError }; active.delete(child);
    while (pending.length) pending.shift()(null);
    fs.closeSync(stdout); fs.closeSync(stderr); resolve(status);
  }));
  child.closed = closed;
  const output = (fd, chunk) => {
    try { fs.writeSync(fd, chunk); } catch (error) {
      spawnError = error.message;
      try { process.kill(-child.pid, 'SIGTERM'); } catch {}
    }
  };
  child.stdout.on('data', chunk => output(stdout, chunk));
  child.stderr.on('data', chunk => output(stderr, chunk));
  readline.createInterface({ input: child.stdout }).on('line', line => {
    if (pending.length) pending.shift()(line); else lines.push(line);
  });
  const next = () => lines.length ? Promise.resolve(lines.shift())
    : ended ? Promise.resolve(null) : new Promise(resolve => pending.push(resolve));
  async function ready(iteration) {
    while (protocol.ready < iteration) {
      const line = await next();
      assert.notEqual(line, null, `${id}: closed before ${iteration}: ${JSON.stringify(status)}`);
      protocol.consume(line);
    }
    assert.equal(protocol.ready, iteration);
  }
  const lane = { id, protocol };
  fs.writeFileSync(`${dir}/${id}.scheduler.jsonl`, '', { flag: 'wx' });
  lane.run = async block => {
    assert.equal(protocol.ready, block.start); assert.equal(protocol.samples.length, block.start);
    const before = await snapshot(protocol.pid, block.start);
    assert.equal(before.schedstats, schedstats);
    const started = process.hrtime.bigint();
    child.stdin.write(Array.from({ length: block.count }, (_, i) => `${block.start + i}\n`).join(''));
    const end = block.start + block.count;
    if (end < 1004) await ready(end);
    else {
      child.stdin.end();
      for (;;) { const line = await next(); if (line === null) break; protocol.consume(line); }
      const result = await closed;
      const text = fs.readFileSync(`${dir}/${id}.stdout`, 'utf8') + '\n' + fs.readFileSync(`${dir}/${id}.stderr`, 'utf8');
      fs.writeFileSync(`${dir}/${id}.log`, text);
      const diagnostics = (text.match(/(^|\s)(ERROR|WARN)(\s|$)|panicked at|Error:/gm) || []).length;
      fs.appendFileSync(`${dir}/results.jsonl`, JSON.stringify({ id, ...result, diagnostics }) + '\n');
      assert.equal(result.code, 0, id); assert.equal(result.signal, null, id);
      assert.equal(result.spawnError, undefined, id); assert.equal(diagnostics, 0, id);
    }
    const finished = process.hrtime.bigint(), samples = protocol.take(block);
    const after = end < 1004 ? await snapshot(protocol.pid, end) : null;
    fs.appendFileSync(`${dir}/${id}.scheduler.jsonl`, JSON.stringify({ ...block, before, after,
      delta: after ? difference(before, after) : null }) + '\n');
    return { samples, interval: { side, start_ns: Number(started), end_ns: Number(finished),
      update_ns: samples.reduce((sum, sample) => sum + sample.ns, 0) } };
  };
  await ready(0);
  assert(protocol.config && protocol.pid);
  identity(lane, 'ready-0');
  return lane;
}
try {
  let poolThreads;
  for (const pair of matrix) {
    pressure(`${pair.id}-before`);
    const lanes = {};
    for (const side of pair.reverse ? ['right', 'left'] : ['left', 'right']) lanes[side] = await openLane(pair, side);
    poolThreads ??= lanes.left.protocol.config.pool_threads;
    assert.equal(lanes.left.protocol.config.pool_threads, poolThreads);
    assert.equal(lanes.right.protocol.config.pool_threads, poolThreads);
    fs.writeFileSync(`${dir}/${pair.id}.jsonl`, '');
    fs.writeFileSync(`${dir}/${pair.id}.blocks.jsonl`, '');
    const plan = blockPlan(pair.name, pair.reverse);
    for (const block of plan) {
      if (block.start === 504) {
        for (const lane of Object.values(lanes)) identity(lane, 'ready-504');
        pressure(`${pair.id}-midpoint`);
      }
      if (block === plan.at(-1)) for (const lane of Object.values(lanes)) identity(lane, 'ready-last-block');
      const results = {}, intervals = [];
      for (const side of block.first ? ['right', 'left'] : ['left', 'right']) {
        const result = await lanes[side].run(block); results[side] = result.samples; intervals.push(result.interval);
      }
      assert(intervals[0].end_ns <= intervals[1].start_ns);
      const rows = results.left.map((left, offset) => {
        const right = results.right[offset]; assert.equal(left.clock, right.clock);
        assert.equal(left.iteration, right.iteration);
        return { iteration: left.iteration, block: block.block, offset, first: block.first,
          clock: left.clock, left_ns: left.ns, right_ns: right.ns };
      });
      fs.appendFileSync(`${dir}/${pair.id}.jsonl`, rows.map(row => JSON.stringify(row)).join('\n') + '\n');
      fs.appendFileSync(`${dir}/${pair.id}.blocks.jsonl`, JSON.stringify({ ...block, intervals }) + '\n');
      if (block.block === -1) for (const lane of Object.values(lanes)) identity(lane, 'ready-4');
    }
    pressure(`${pair.id}-after`);
    fs.appendFileSync(`${dir}/progress.log`, `${new Date().toISOString()} ${pair.id} complete\n`);
    console.log(`${pair.id} complete`);
  }
  for (const [path, expected] of Object.entries(inputs)) assert.equal(hash(path), expected, path);
  assert.equal(hash(`${dir}/run`), inputs[`${base}/before`]);
  for (const [type, cpus] of Object.entries(cpuMaps))
    assert.equal(fs.readFileSync(`/sys/bus/event_source/devices/cpu_${type}/cpus`, 'utf8').trim(), cpus);
  assert.equal(fs.readFileSync('/proc/sys/kernel/sched_schedstats', 'utf8').trim(), schedstats);
  write('complete.json', { processes: 20, updates: 20000, warmups: 80, blockSizes, poolThreads, inputsVerified: true });
} catch (error) {
  fs.appendFileSync(`${dir}/progress.log`, `${error.stack}\n`);
  await stopOwned(); throw error;
}
