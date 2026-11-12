import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { parseThread, difference, snapshot, inputBlocked } from './scheduler.mjs';

function raw() {
  const fields = Array(50).fill('0'); fields[0] = 'S'; fields[19] = '123'; fields[36] = '16';
  return { stat: `42 (main ) thread) ${fields.join(' ')}\n`, schedstat: '1000 321 3\n',
    sched: 'se.nr_migrations : 2\nnr_voluntary_switches : 3\nnr_involuntary_switches : 4\n',
    status: 'Cpus_allowed_list:\t0-23\n' };
}
function snapshots() {
  const thread = parseThread(42, raw());
  return [{ pid: 42, ready: 4, schedstats: '0', start_ns: 10000, end_ns: 10100, threads: [thread] },
    { pid: 42, ready: 254, schedstats: '0', start_ns: 14000, end_ns: 14100,
      threads: [{ ...thread, cpu_ns: 3000, voluntary: 4, raw_wait_ns: 999 }] }];
}

test('thread parser preserves identity, CPU, affinity and raw counters', () => {
  assert.deepEqual(parseThread(42, raw()), { tid: 42, state: 'S', start: 123, cpu: 16, allowed: '0-23',
    cpu_ns: 1000, raw_wait_ns: 321, timeslices: 3, migrations: 2, voluntary: 3, involuntary: 4 });
});
test('disabled runqueue accounting is unavailable, not zero delay', () => {
  const [before, after] = snapshots(), delta = difference(before, after);
  assert.equal(delta.main.cpu_ns, 2000); assert.equal(delta.main.wait_ns, null);
  assert.equal(delta.window_ns, 4100); assert.equal(delta.capture_ns, 200);
  assert.equal(delta.main.voluntary, 1); assert.equal(delta.worker_cpu_ns, 0);
  before.schedstats = after.schedstats = '1';
  assert.equal(difference(before, after).main.wait_ns, 678);
});
test('snapshot counters reject identity, affinity, scheduling and chronology changes', () => {
  for (const mutate of [
    a => { a.pid++; }, a => { a.schedstats = '1'; }, a => { a.start_ns = 0; },
    a => { a.threads = []; }, a => { a.threads[0].start++; }, a => { a.threads[0].allowed = '0'; },
    a => { a.threads[0].cpu_ns = 1; }, a => { a.threads[0].cpu_ns = 99999; },
    a => { a.threads[0].migrations = 0; }, a => { a.threads[0].cpu = 1; },
  ]) { const [before, after] = snapshots(); mutate(after); assert.throws(() => difference(before, after)); }
});
test('migration observations retain both endpoints without guessing residency', () => {
  const [before, after] = snapshots(); after.threads[0].cpu = 2; after.threads[0].migrations++;
  const delta = difference(before, after);
  assert.equal(delta.main.migrations, 1); assert.equal(delta.main.from_cpu, 16); assert.equal(delta.main.to_cpu, 2);
});
test('worker CPU is kept separate from main-thread execution', () => {
  const [before, after] = snapshots();
  before.threads.push({ ...before.threads[0], tid: 43, cpu_ns: 500 });
  after.threads.push({ ...before.threads[1], cpu_ns: 3500 });
  const delta = difference(before, after);
  assert.equal(delta.main.cpu_ns, 2000); assert.equal(delta.worker_cpu_ns, 3000);
  assert.equal(delta.threads.length, 2);
});
test('thread parser rejects missing, truncated, negative and unsafe counters', () => {
  assert.throws(() => parseThread(43, raw()));
  for (const mutate of [
    r => { r.stat = '42 (main) S'; }, r => { r.schedstat = '1 2'; },
    r => { r.schedstat = '-1 2 3'; }, r => { r.schedstat = '9007199254740992 2 3'; },
    r => { r.sched = ''; }, r => { r.sched += 'se.nr_migrations : 1\n'; }, r => { r.status = ''; },
  ]) { const r = raw(); mutate(r); assert.throws(() => parseThread(42, r)); }
});

test('input readiness accepts inherited pipes and Unix stream sockets only', () => {
  for (const name of ['pipe_read', 'anon_pipe_read', 'unix_stream_read_generic']) {
    assert(inputBlocked('S', name)); assert(!inputBlocked('R', name));
  }
  for (const name of ['futex_wait_queue', 'ep_poll', '0', '']) assert(!inputBlocked('S', name));
});

test('snapshot observes an owned child blocked on Node-provided stdin', async () => {
  const child = spawn('/run/current-system/sw/bin/bash', ['-c', "printf 'ready\\n'; IFS= read -r line; test \"$line\" = finish"],
    { stdio: ['pipe', 'pipe', 'pipe'] });
  const closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
  try {
    const ready = await new Promise((resolve, reject) => {
      child.stdout.once('data', resolve); child.once('error', reject);
      child.once('close', () => reject(new Error('child exited before readiness')));
    });
    assert.equal(ready.toString(), 'ready\n');
    const value = await snapshot(child.pid, 0);
    assert.equal(value.pid, child.pid); assert.equal(value.threads.length, 1);
    assert.equal(value.threads[0].state, 'S');
    child.stdin.end('finish\n');
    assert.deepEqual(await closed, { code: 0, signal: null });
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
    await closed;
  }
});
