import assert from 'node:assert/strict';
import test from 'node:test';
import { names, blockPlan, StepProtocol } from './protocol.mjs';

function sample(iteration) {
  return `${iteration < 4 ? 'seek_warmup' : 'seek'}_sample index=${iteration < 4 ? iteration : iteration - 4} clock=${iteration / 1000} update_ns=123`;
}

test('block plans retain warmups and every measured update with balanced first orders', () => {
  for (const name of names) for (const reverse of [false, true]) {
    const plan = blockPlan(name, reverse);
    assert.deepEqual(plan[0], { block: -1, start: 0, count: 4, first: Number(reverse) });
    assert.deepEqual(plan.flatMap(b => Array.from({ length: b.count }, (_, i) => b.start + i)),
      Array.from({ length: 1004 }, (_, i) => i));
    assert.deepEqual(plan.slice(1, 5).map(b => b.first), reverse ? [1, 0, 0, 1] : [0, 1, 1, 0]);
    if (plan.length > 5) assert.deepEqual(plan.slice(5, 9).map(b => b.first), plan.slice(1, 5).map(b => b.first));
    for (const first of [0, 1]) assert.equal(plan.slice(1).filter(b => b.first === first).reduce((sum, b) => sum + b.count, 0), 500);
    const other = blockPlan(name, !reverse);
    assert(plan.every((b, i) => b.first !== other[i].first));
  }
});

test('buffered blocks require every ready marker and sample before advancing', () => {
  for (const name of names) {
    const protocol = new StepProtocol();
    protocol.consume('owned_app_pid=1234');
    protocol.consume('seek_step_config samples=1000 warmup=4 pool_threads=16');
    protocol.consume('seek_step_ready iteration=0');
    for (const block of blockPlan(name, false)) {
      for (let i = block.start; i < block.start + block.count; i++) {
        protocol.consume(sample(i));
        if (i < 1003) protocol.consume(`seek_step_ready iteration=${i + 1}`);
      }
      const values = protocol.take(block);
      assert.equal(values.length, block.count);
      assert.equal(values[0].iteration, block.start);
    }
    assert.equal(protocol.samples.length, 1004);
    assert.equal(protocol.ready, 1003);
  }
});

test('missing, duplicated and out-of-order protocol events are rejected', () => {
  assert.throws(() => new StepProtocol().consume(sample(0)), /matching readiness/);
  assert.throws(() => new StepProtocol().consume('seek_step_ready iteration=1'), /readiness index/);
  const ready = () => {
    const p = new StepProtocol(); p.consume('seek_step_ready iteration=0'); return p;
  };
  assert.throws(() => ready().consume('seek_step_ready iteration=1'), /preceding sample/);
  assert.throws(() => ready().consume(sample(4)), /sample phase/);
  assert.throws(() => ready().consume(sample(1)));
  assert.throws(() => ready().consume(sample(0).replace('update_ns=123', 'update_ns=NaN')));
  assert.throws(() => ready().consume(sample(0).replace('update_ns=123', 'update_ns=0')));
  const p = ready(); p.consume(sample(0));
  assert.throws(() => p.consume(sample(0)), /matching readiness/);
  assert.throws(() => p.take({ start: 0, count: 4 }), /block sample count/);
});

test('unknown workload and non-boolean order are rejected', () => {
  assert.throws(() => blockPlan('unknown', false));
  assert.throws(() => blockPlan('bird', 1));
});
