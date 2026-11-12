# Sustained-block editor controls

The normal-keyed tangent candidate passes its declared kernel gate, but prior
single-update alternating editor runs fail identical-binary controls. This
experiment tests a different pacing method before building that candidate into
the editor. It does not reinterpret or replace the old failed results.

Use the unchanged saved original editor, SHA-256
`8297f07802f819c4866e8faa4dd63cd85f4ae5d07fbf689223427b9da08eced1`,
whose input manifest verifies and runtime source matches the current checkout.
The existing stepped-input contract accepts sequential buffered command lines;
no Rust/runtime changes, affinity changes, desktop controls or quality changes.

## Protocol and gates declared before measurement

- Five workloads: bird, cow, robot arm, GPU morph and material subsets.
- Two fresh reversed-startup/order pairs per workload: ten pairs, twenty
  processes, 20,000 measured updates and 80 retained warmups.
- Four warmups form one block. Measured blocks contain 20 updates for bird/cow
  and 250 for arm/morph/subsets. The sizes are fixed from the existing relative
  workload costs, not selected from this experiment's results.
- Feed every command in a block once, then wait for its ending readiness marker
  before advancing the other process. Alternate process blocks in ABBA order;
  reverse that order in the second fresh pair. All 1,000 authored-range clocks
  remain in order. Retain every first/resumed update; discard no block starts.
- Analyze both first-block orders separately: 500 equal-clock update pairs per
  stratum. Require median paired ratios within **1%** of unity and p95 ratios
  within **5%**, in every stratum of both fresh pairs for all five workloads.
- Require zero process/diagnostic failures, every ready marker/clock/raw sample,
  matching payload/work/animation-operation counts and raw-to-CSV quantiles.
  Verify actual executable hashes, PID start identities and unchanged default
  main-thread affinity at startup, after warmup, midpoint and before the last
  block. Verify the reported compute-pool count matches across processes.
- Preserve opening, idle, memory, raw tails, per-block wall intervals and host
  load. No owned builds/captures overlap measurement. Leave unrelated work and
  affinity alone. Reverify inputs and assets after the run.
- Test negative controls for clock, block schedule, payload and timing checks.
  A failed control prevents candidate performance acceptance; do not adjust
  sizes/limits or repeat this unchanged to obtain a pass.

This is headless editor CPU-update evidence, not native frame cadence or GPU
rendering. Only stable controls permit a subsequent comparison with the exact
normal-keyed candidate under separately recorded application/tail/memory gates.
The cow kernel p95 increase and all broader acceptance gaps remain open.

Evidence: `/tmp/bevy-optimization-editor-blocks.kvoNGn/`.
