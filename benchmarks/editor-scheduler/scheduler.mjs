import fs from 'node:fs';
import assert from 'node:assert/strict';
import { setTimeout } from 'node:timers/promises';

const integer = value => {
  assert(/^\d+$/.test(value), 'invalid scheduler integer');
  const number = Number(value);
  assert(Number.isSafeInteger(number), 'unsafe scheduler integer');
  return number;
};
const now = () => Number(process.hrtime.bigint());

export function parseThread(tid, raw) {
  const end = raw.stat.lastIndexOf(')');
  assert(end > 0 && raw.stat.startsWith(`${tid} (`), 'thread identity mismatch');
  const stat = raw.stat.slice(end + 2).trim().split(/\s+/);
  assert(stat.length >= 37, 'truncated thread stat');
  const times = raw.schedstat.trim().split(/\s+/);
  assert.equal(times.length, 3, 'truncated scheduler counters');
  const counter = key => {
    const lines = raw.sched.split('\n').filter(line => line.split(':')[0].trim() === key);
    assert.equal(lines.length, 1, `missing scheduler counter ${key}`);
    return integer(lines[0].split(':')[1].trim());
  };
  const allowed = raw.status.match(/^Cpus_allowed_list:\s*([^\n]+)$/m);
  assert(allowed, 'missing thread affinity');
  return { tid, state: stat[0], start: integer(stat[19]), cpu: integer(stat[36]),
    allowed: allowed[1].trim(), cpu_ns: integer(times[0]), raw_wait_ns: integer(times[1]),
    timeslices: integer(times[2]), migrations: counter('se.nr_migrations'),
    voluntary: counter('nr_voluntary_switches'), involuntary: counter('nr_involuntary_switches') };
}

export async function snapshot(pid, ready) {
  const proc = `/proc/${pid}`, main = `${proc}/task/${pid}`;
  let attempts = 0;
  while (true) {
    const stat = fs.readFileSync(`${main}/stat`, 'utf8');
    const state = stat.slice(stat.lastIndexOf(')') + 2).split(' ')[0];
    const wchan = fs.readFileSync(`${main}/wchan`, 'utf8').trim();
    if (state === 'S' && /^(anon_)?pipe_read$/.test(wchan)) break;
    assert(++attempts < 1000, `owned main thread not input-blocked: ${state} ${wchan}`);
    await setTimeout(1);
  }
  const start_ns = now(), schedstats = fs.readFileSync('/proc/sys/kernel/sched_schedstats', 'utf8').trim();
  assert(['0', '1'].includes(schedstats));
  const tids = fs.readdirSync(`${proc}/task`).map(integer).sort((a, b) => a - b);
  const threads = tids.map(tid => {
    const path = `${proc}/task/${tid}`;
    const raw = Object.fromEntries(['stat', 'schedstat', 'sched', 'status'].map(name => [name, fs.readFileSync(`${path}/${name}`, 'utf8')]));
    return { ...parseThread(tid, raw), raw };
  });
  assert.deepEqual(fs.readdirSync(`${proc}/task`).map(integer).sort((a, b) => a - b), tids);
  assert.equal(threads.find(t => t.tid === pid)?.state, 'S');
  return { pid, ready, schedstats, start_ns, end_ns: now(), threads };
}

export function difference(before, after) {
  assert.equal(before.pid, after.pid, 'snapshot process mismatch');
  assert.equal(before.schedstats, after.schedstats, 'scheduler accounting changed');
  assert(before.end_ns <= after.start_ns, 'snapshot intervals overlap');
  assert(before.start_ns <= before.end_ns && after.start_ns <= after.end_ns);
  assert.deepEqual(before.threads.map(t => t.tid), after.threads.map(t => t.tid), 'thread set changed');
  const threads = before.threads.map((a, i) => {
    const b = after.threads[i];
    assert.equal(a.start, b.start, 'thread identity changed');
    assert.equal(a.allowed, b.allowed, 'thread affinity changed');
    const counters = Object.fromEntries(['cpu_ns', 'migrations', 'voluntary', 'involuntary'].map(key => {
      assert(b[key] >= a[key], `nonmonotonic scheduler counter ${key}`);
      return [key, b[key] - a[key]];
    }));
    let wait_ns = null;
    if (before.schedstats === '1') {
      assert(b.raw_wait_ns >= a.raw_wait_ns && b.timeslices >= a.timeslices);
      wait_ns = b.raw_wait_ns - a.raw_wait_ns;
    }
    if (counters.migrations === 0) assert.equal(a.cpu, b.cpu, 'CPU changed without migration');
    return { tid: a.tid, ...counters, wait_ns, from_cpu: a.cpu, to_cpu: b.cpu };
  });
  const main = threads.find(t => t.tid === before.pid);
  assert(main, 'missing main thread');
  const window_ns = after.end_ns - before.start_ns;
  assert(main.cpu_ns <= window_ns, 'main CPU exceeds external window');
  return { window_ns, capture_ns: before.end_ns - before.start_ns + after.end_ns - after.start_ns,
    main, worker_cpu_ns: threads.filter(t => t.tid !== before.pid).reduce((sum, t) => sum + t.cpu_ns, 0), threads };
}
