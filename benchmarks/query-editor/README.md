# Integrated animation-query gate

Compare the saved original release editor with the isolated stage-query owner,
including its upstream patches. The original executable is the traced editor
in `/tmp/bevy-optimization-animation-access.tzNL5k/editor`; its manifest must
verify and its runtime source must match the main checkout before measurement.
This is a headless full-editor CPU-update gate, not rendered FPS acceptance.

Before measurement, require these limits without adjusting them to the result:

- Identical-executable controls: paired median ratios within 1% of unity and
  p95 ratios within 5%, in each first-update stratum and fresh-process order.
- Cache enabled: at least a 1% paired-median reduction for bird and cow, and
  no more than 5% p95 overhead. Robot arm, GPU morph and material-subset controls
  must have no more than 5% paired-median or p95 overhead.
- Patched direct fallback (`max_queries = 0`): no more than 5% paired-median
  or p95 overhead on bird/cow versus the unmodified dependency/editor.
- Every process: zero application diagnostics, correct document/readiness and
  clocks, matching payload and non-timing work counts. Retain all failures.
- Cached bird/cow: one/two retained queries, three/six retained sources, bounded
  metadata/source storage, and a reused query for each measured construction
  request. Direct fallback must retain no query/source storage.

Use 1,000 unique authored-range clocks and four warmups. Each pair runs separate
processes, one update at a time in ABBA order; reverse opening and update order
in a second fresh pair. Both executables launch through the same pathname.
The five workloads run identical controls and cached comparisons; bird/cow
also run direct-fallback comparisons: 24 pairs, 48 processes, 48,000 measured
updates and 192 warmups. Preserve raw p95/max and opening/idle observations.

Do not overlap owned builds or captures. Record host load and default affinity;
leave unrelated jobs and affinity unchanged. Validate inputs again afterward.
Run negative controls against clock, cache-budget and performance checks.

Passing this gate still leaves actual mesh/image equivalence, native rendering,
frame pacing, browser checks and broader library optimization acceptance open.
