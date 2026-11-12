# Integrated query reuse: performance gate failed

The stage-query owner remains an isolated candidate. Its storage, lookup,
clock, payload-count and work-count checks pass on the real editor workloads.
The declared whole-editor performance gate fails: 16 of 20 identical-control
strata and 8 of 20 cached-candidate strata fail. The eight direct-fallback
strata meet their numeric limits, but the failed controls prevent overall
performance acceptance. No production source/dependency change or FPS gain is
claimed.

## Source and checks

Evidence: `/tmp/bevy-optimization-query-editor.Xnxbam/`.

The original executable is the saved animation-access editor. Its input
manifest verifies, and its runtime source has no differences from the main
checkout. The candidate is the isolated viewer at `eb99457`, using upstream
`c517437f`. Root `a4047df` records the
[gate and harness](query-editor/README.md) before measurement. Root `9f967ec`
retains the [usage patch](patches/viewer-query-usage.patch), applied after
the [owner patch](patches/viewer-animation-query-owner.patch). A fresh replay
matches all six changed viewer files byte-for-byte. The upstream five-patch
chain and metadata accounting are recorded in the
[owner report](stage-animation-queries.md).

The candidate exposes `AnimationQueryUsage` through `LiveStage`. Counts
saturate; a hit describes reuse of a query handle, not an attribute-source
memo hit. The benchmark prints cumulative owner counters and retained storage
outside the timed update. Its cache-disabled mode sets `max_queries = 0` and
uses the normal direct query fallback.

The candidate passes 761 all-feature library tests with 34 ignored cases,
753 no-default-feature library tests with 25 ignored cases, strict
workspace/all-target/all-feature Clippy, all 16 editor-benchmark tests and a
release benchmark build. Test configurations overlap; the ignored cases did
not run. The new usage test covers counters, storage, clearing, refusal and
saturation. `build-checks.exit = 0` records the terminal chain.

Saved executable SHA-256 identities:

```text
before 8297f07802f819c4866e8faa4dd63cd85f4ae5d07fbf689223427b9da08eced1
after  9a226b2dcb95407f88c8ddbbac241f8ba9e4e1163b7641820fa339fc91638b35
```

## Real-editor verification

All 48 processes exit zero with zero application diagnostics. They execute
48,000 measured updates and 192 warmups in 24 paired runs. Each process runs
1,000 unique authored-range clocks. ABBA update order and reversed fresh-process
opening/order supply two 500-pair first-update strata per comparison. The
driver serializes the updates and launches both executable versions through
the same pathname. `/proc` executable hashes, process start identities, main
thread affinity and the common compute-pool count are checked at the recorded
boundaries.

The independent analyzer validates every readiness step, clock, raw duration,
warmup sum, CSV percentile, payload count, image-buffer capacity, non-timing
work counter and animation-operation row. These are not bitwise mesh or
rendered-image comparisons. All source/binary inputs and the 743-entry asset
manifest verify after measurement.

Cached bird/cow processes preserve these owner values from warmup to the end
of measured seeks:

| Scene | Queries | Metadata bytes | Sources | Source bytes | Measured hits |
| --- | ---: | ---: | ---: | ---: | ---: |
| Bird | 1 | 34,657 | 3 | 1,658,967 | 1,000 |
| Cow | 2 | 15,035 | 6 | 2,916,090 | 2,000 |

Each measured query-construction request reuses its retained handle, with no
new admission, refusal or owner clearing during that interval. These charges
remain within 128 queries/1 MiB metadata and 256 sources/8 MiB source storage.
All fallback processes retain zero queries, metadata slots or sources; every
construction request takes the direct path. The accounting exclusions in the
owner report still apply. Query-budget charges are not whole-process RSS.

## Timing result

Ratios below compare the candidate with the original, across both fresh pairs
and first-update strata. A value below one is lower measured CPU cost; these
observations do not establish an accepted gain.

| Cached workload | Paired median ratio range | p95 ratio range | Failed strata |
| --- | ---: | ---: | ---: |
| Bird | 0.9726–1.0041 | 0.9579–1.0045 | 2 / 4 |
| Cow | 0.9835–0.9952 | 0.9730–1.0083 | 2 / 4 |
| Robot arm | 0.9482–1.0440 | 0.9268–1.1883 | 1 / 4 |
| GPU morph | 0.9439–1.0464 | 0.9585–1.1204 | 2 / 4 |
| Material subsets | 0.9429–1.0354 | 0.9395–1.0808 | 1 / 4 |

Bird and cow each miss the 1% reduction requirement in two strata. The other
candidate failures exceed the 5% p95 overhead bound. Identical-binary controls
fail three bird, one cow and all four arm, morph and subset strata. Ratios
depend strongly on which process updates first even with the same executable;
the control failures cannot be attributed to query reuse.

The direct-fallback median ratios are 0.9883–1.0176 for bird and 0.9911–1.0035
for cow, with p95 ratios no greater than 1.0412. Their numeric bounds pass;
this does not waive the control failures or prove the default path universally
regression-free. Oversized borrowed-source refusal is a separate untested
editor case.

Starting one-minute host load ranges from 4.04 to 10.66. Unrelated work is
recorded and left alone; no owned build/capture overlaps measurement. Opening,
idle, warmups, full-run tails, process RSS and raw paired samples remain in the
evidence. Headless updates exclude UI and GPU work and cannot establish native
60 Hz.

`driver.exit = 0` records completed processes. `analysis.exit = 1` preserves
all 24 timing failures. `verification.exit = 0` verifies negative controls:
an altered clock and excessive source charge are rejected, and doubling the
validated non-control timings makes all 28 affected strata fail. It does not
turn the original performance failure into a pass.

## Next work

Do not adopt or rerun this query candidate unchanged. Larger mesh/tangent costs
remain the primary optimization opportunity from the existing profiles. A
full-editor measurement method with stable identical controls is also still
needed before accepting small changes. Preserve this failed run while pursuing
those issues; do not merge first-update strata, relax limits or label the
schema-kernel gains as application gains. Native visual/lifecycle/browser
acceptance and all broader workstreams remain open.
