import fs from 'node:fs';
import os from 'node:os';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import readline from 'node:readline';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const base = process.argv[2], cwd = process.cwd();
assert(base && base.startsWith('/'));
const dir = `${base}/perf`;
fs.mkdirSync(dir);
const launcher = fileURLToPath(new URL('launch.sh', import.meta.url));
const hash = path => createHash('sha256').update(fs.readFileSync(path)).digest('hex');
const fields = line => Object.fromEntries([...line.matchAll(/(\w+)=([^\s]+)/g)].map(m => [m[1], m[2]]));
const names = ['bird', 'cow', 'arm', 'morph', 'subsets'];
const assets = {
  bird: 'assets/external/hummingbird.usdz', cow: 'assets/external/cows/cow_eating.usda',
  arm: 'assets/external/universal_robots/ur10_pick.usda',
  morph: 'assets/skel_morph_blended_animated.usda', subsets: 'assets/material_subsets.usda',
};
const matrix = [];
for (const reverse of [false, true]) {
  for (const phase of reverse ? ['cached', 'bypass', 'identical'] : ['identical', 'bypass', 'cached']) {
    const selected = phase === 'bypass' ? names.slice(0, 2) : names;
    for (const name of reverse ? [...selected].reverse() : selected) {
      matrix.push({ id: `${phase}-${name}-${Number(reverse)}`, phase, name, reverse });
    }
  }
}
const inputs = Object.fromEntries([
  `${base}/before`, `${base}/after`, launcher, fileURLToPath(import.meta.url),
  'Cargo.lock', ...Object.values(assets),
].map(file => [file, hash(file)]));
const write = (name, value) => fs.writeFileSync(`${dir}/${name}`, JSON.stringify(value, null, 2) + '\n');
write('matrix.json', matrix); write('inputs.json', inputs);
write('environment.json', {
  date: new Date().toISOString(), cwd, assets, kernel: os.release(), cpu: os.cpus()[0].model,
  controllerStatus: fs.readFileSync('/proc/self/status', 'utf8'),
  scope: 'paired-headless-editor-update; default-affinity; default-eager-residency; no-owned-builds',
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
  const proc = `/proc/${lane.pid}`;
  const status = fs.readFileSync(`${proc}/status`, 'utf8');
  const value = {
    pid: lane.pid, label, sha256: hash(`${proc}/exe`),
    stat: fs.readFileSync(`${proc}/stat`, 'utf8'),
    allowed: status.match(/^Cpus_allowed_list:\s*(.+)$/m)[1],
  };
  assert.equal(value.sha256, inputs[`${base}/${lane.version}`]);
  assert.equal(value.allowed, fs.readFileSync('/proc/self/status', 'utf8').match(/^Cpus_allowed_list:\s*(.+)$/m)[1]);
  fs.appendFileSync(`${dir}/${lane.id}.identity.jsonl`, JSON.stringify(value) + '\n');
}
async function openLane(pair, side) {
  const version = pair.phase !== 'identical' && side === 'right' ? 'after' : 'before';
  const id = `${pair.id}-${side}`;
  fs.copyFileSync(`${base}/${version}`, `${dir}/run.next`);
  fs.renameSync(`${dir}/run.next`, `${dir}/run`);
  assert.equal(hash(`${dir}/run`), inputs[`${base}/${version}`]);
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('USD_')));
  Object.assign(env, {
    USD_BENCH_STEPPED: '1', USD_PROFILE_ROUTES: '1', USD_PROFILE_IMAGES: '1',
    USD_PROFILE_ANIMATION_READS: '1', USD_PROFILE_QUERY_CACHE: '1',
    USD_BENCH_QUERY_CACHE: pair.phase === 'bypass' ? '0' : '1',
  });
  const stdout = fs.openSync(`${dir}/${id}.stdout`, 'wx');
  const stderr = fs.openSync(`${dir}/${id}.stderr`, 'wx');
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
    fs.closeSync(stdout); fs.closeSync(stderr);
    resolve(status);
  }));
  child.closed = closed;
  child.stdout.on('data', chunk => fs.writeSync(stdout, chunk));
  child.stderr.on('data', chunk => fs.writeSync(stderr, chunk));
  readline.createInterface({ input: child.stdout }).on('line', line => {
    if (pending.length) pending.shift()(line); else lines.push(line);
  });
  const next = () => lines.length ? Promise.resolve(lines.shift())
    : ended ? Promise.resolve(null) : new Promise(resolve => pending.push(resolve));
  const lane = { id, version, config: null, sample: null, pid: null };
  function consume(line) {
    if (line.startsWith('owned_app_pid=')) { assert.equal(lane.pid, null); lane.pid = Number(fields(line).owned_app_pid); }
    if (line.startsWith('seek_step_config ')) { assert.equal(lane.config, null); lane.config = fields(line); }
    if (/^seek(?:_warmup)?_sample /.test(line)) { assert.equal(lane.sample, null); lane.sample = fields(line); }
  }
  async function ready(iteration) {
    for (;;) {
      const line = await next();
      assert.notEqual(line, null, `${id}: closed before ${iteration}: ${JSON.stringify(status)}`);
      consume(line);
      if (line.startsWith('seek_step_ready ')) {
        assert.equal(Number(fields(line).iteration), iteration);
        return;
      }
    }
  }
  lane.step = async iteration => {
    lane.sample = null;
    child.stdin.write(`${iteration}\n`);
    if (iteration < 1003) {
      await ready(iteration + 1);
      if (iteration === 1002) identity(lane, 'ready-1003');
    } else {
      child.stdin.end();
      for (;;) { const line = await next(); if (line === null) break; consume(line); }
      const result = await closed;
      const text = fs.readFileSync(`${dir}/${id}.stdout`, 'utf8') + '\n' + fs.readFileSync(`${dir}/${id}.stderr`, 'utf8');
      fs.writeFileSync(`${dir}/${id}.log`, text);
      const diagnostics = (text.match(/(^|\s)(ERROR|WARN)(\s|$)|panicked at|Error:/gm) || []).length;
      fs.appendFileSync(`${dir}/results.jsonl`, JSON.stringify({ id, version, ...result, diagnostics }) + '\n');
      assert.equal(result.code, 0, id); assert.equal(result.signal, null, id);
      assert.equal(result.spawnError, undefined, id); assert.equal(diagnostics, 0, id);
    }
    assert(lane.sample, `${id}: sample missing`);
    assert.equal(Number(lane.sample.index), iteration < 4 ? iteration : iteration - 4);
    assert(Number(lane.sample.update_ns) > 0);
    return lane.sample;
  };
  await ready(0);
  assert.equal(lane.config.samples, '1000'); assert.equal(lane.config.warmup, '4');
  assert(Number(lane.config.pool_threads) > 1); assert(Number.isSafeInteger(lane.pid));
  identity(lane, 'ready-0');
  return lane;
}
try {
  let poolThreads;
  for (const pair of matrix) {
    pressure(`${pair.id}-before`);
    const lanes = {};
    for (const side of pair.reverse ? ['right', 'left'] : ['left', 'right']) lanes[side] = await openLane(pair, side);
    poolThreads ??= lanes.left.config.pool_threads;
    assert.equal(lanes.left.config.pool_threads, poolThreads);
    assert.equal(lanes.right.config.pool_threads, poolThreads);
    const output = fs.openSync(`${dir}/${pair.id}.jsonl`, 'wx');
    for (let iteration = 0; iteration < 1004; iteration++) {
      const first = Number([1, 2].includes(iteration % 4) !== pair.reverse);
      const sample = {};
      for (const side of first ? ['right', 'left'] : ['left', 'right']) sample[side] = await lanes[side].step(iteration);
      assert.equal(sample.left.clock, sample.right.clock);
      fs.writeSync(output, JSON.stringify({ iteration, first, clock: Number(sample.left.clock),
        left_ns: Number(sample.left.update_ns), right_ns: Number(sample.right.update_ns) }) + '\n');
    }
    fs.closeSync(output);
    pressure(`${pair.id}-after`);
    fs.appendFileSync(`${dir}/progress.log`, `${new Date().toISOString()} ${pair.id} complete\n`);
    console.log(`${pair.id} complete`);
  }
  for (const [path, expected] of Object.entries(inputs)) assert.equal(hash(path), expected, path);
  write('complete.json', { processes: 48, updates: 48000, warmups: 192, poolThreads, inputsVerified: true });
} catch (error) {
  fs.appendFileSync(`${dir}/progress.log`, `${error.stack}\n`);
  const remaining = [...active];
  for (const child of remaining) { try { process.kill(-child.pid, 'SIGTERM'); } catch {} }
  await Promise.all(remaining.map(child => child.closed));
  throw error;
}
