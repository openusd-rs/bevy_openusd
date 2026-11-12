import assert from 'node:assert/strict';

export const names = ['bird', 'cow', 'arm', 'morph', 'subsets'];
export const blockSizes = { bird: 20, cow: 20, arm: 250, morph: 250, subsets: 250 };
export const fields = line => Object.fromEntries([...line.matchAll(/(\w+)=([^\s]+)/g)].map(m => [m[1], m[2]]));

export function blockPlan(name, reverse) {
  assert(names.includes(name));
  assert.equal(typeof reverse, 'boolean');
  const size = blockSizes[name];
  return [{ block: -1, start: 0, count: 4, first: Number(reverse) },
    ...Array.from({ length: 1000 / size }, (_, block) => ({
      block, start: 4 + block * size, count: size,
      first: Number([1, 2].includes(block % 4) !== reverse),
    }))];
}

export class StepProtocol {
  pid = null;
  config = null;
  ready = -1;
  samples = [];

  consume(line) {
    if (line.startsWith('owned_app_pid=')) {
      assert.equal(this.pid, null);
      this.pid = Number(fields(line).owned_app_pid);
      assert(Number.isSafeInteger(this.pid) && this.pid > 1);
    } else if (line.startsWith('seek_step_config ')) {
      assert.equal(this.config, null);
      this.config = fields(line);
      assert.equal(this.config.samples, '1000');
      assert.equal(this.config.warmup, '4');
      assert(Number(this.config.pool_threads) > 1);
    } else if (line.startsWith('seek_step_ready ')) {
      const iteration = Number(fields(line).iteration);
      assert.equal(iteration, this.ready + 1, 'readiness index mismatch');
      assert.equal(this.samples.length, iteration, 'readiness without preceding sample');
      assert(iteration < 1004);
      this.ready = iteration;
    } else if (/^seek(?:_warmup)?_sample /.test(line)) {
      const iteration = this.samples.length, source = fields(line);
      assert.equal(this.ready, iteration, 'sample without matching readiness');
      assert(line.startsWith(iteration < 4 ? 'seek_warmup_sample ' : 'seek_sample '), 'sample phase mismatch');
      assert.equal(Number(source.index), iteration < 4 ? iteration : iteration - 4);
      const clock = Number(source.clock), ns = Number(source.update_ns);
      assert(Number.isFinite(clock));
      assert(Number.isSafeInteger(ns) && ns > 0);
      this.samples.push({ iteration, clock, ns });
    }
  }

  take(block) {
    const end = block.start + block.count;
    assert.equal(this.samples.length, end, 'block sample count mismatch');
    assert.equal(this.ready, Math.min(end, 1003), 'block is not paused at its boundary');
    return this.samples.slice(block.start, end);
  }
}
