# Paired tangent scheduling diagnostic

## Measurement contract

The withdrawn queue's full-scene comparison is inconclusive under large host
load swings; it is not re-enabled. This test-only probe separates scheduling
and MikkTSpace generation from source sampling and the rest of the application.
Production code remains on the original scheduler.

At every clock, the probe samples the same sorted animated prim paths and uses
production GPU eligibility, CPU deformation and mesh assembly to construct owned
tangent inputs. Both schedulers receive independent identical clones, made before
timing, with the originals dropped before either lane runs. Clone order alternates
independently of execution order, covering every combination equally. The legacy
four-prim batches and withdrawn eight-prim/four-worker queues
alternate first within sequential pairs in ABBA order. A second fresh process
reverses that order. Both use one private four-thread pool and the same 16 MiB
worker-input limit, 128-context boundaries and oversized serial fallback.

The timers include batching, dispatch/join, generation and result collection.
They exclude USD reads, eligibility, deformation, assembly, input cloning,
comparison, material routes, asset publication, rendering and GPU work. Caches
are disabled for both lanes; all requested tangent jobs are generated. Results
coexist until full attribute/index/morph byte comparison outside the timers.
This is not a production-global-pool contention test or a playback/FPS result.

Before measurement, use two fresh processes per bird/cow asset, four warmups and
200 distinct authored-range clocks per process. Require complete jobs, zero
tangent failures, byte-exact output and bounded queues/input bytes. Independently
recompute summaries and clocks from raw records. Report paired ratios separately
for legacy-first and queue-first samples in each process. An incremental queue
signal requires median queued/legacy <= 0.85 for cow in both order strata and
both processes, with no >5% bird median-paired regression. Failure does not change
these thresholds. Even a passing kernel signal does not clear the earlier
whole-application failures or authorize default re-enablement.

## Reproduction

`nix develop --command oslo make build-tangent-probe` builds without running.
It explicitly enables Bevy's multi-threaded feature rather than depending on
feature unification from unrelated workspace targets; the probe asserts four
actual pool threads before collecting samples. Library defaults are unchanged.
Save the library test executable named in that build log before measuring.
Run it through `oslo make run-headless-benchmark`, with arguments
`route::preparation::paired_probe::real_asset_paired_tangent_scheduling --ignored --nocapture --test-threads=1`.
Set `USD_TANGENT_ASSET` to an absolute file path, `USD_TANGENT_SAMPLES=200`, and
`USD_TANGENT_REVERSE` to `0` or `1`. Do not compile while timing.

The regular regression compares both probe schedulers against independent serial
MikkTSpace across clocks, sparse >128-context inputs and oversized budgets. The
ignored diagnostic is release-only and requires an explicit asset.

## Harness verification

Hardening passes 853 no-default and 972 all-feature/all-target executions,
strict Clippy, formatting and whitespace checks. The new fixture regression
passes in both configurations; the external-asset diagnostic remains explicitly
ignored until run against a saved release executable. Configurations overlap.
Evidence is retained under `/tmp/bevy-optimization-tangent-paired.ApABK6/`.
No production scheduler, resource defaults or dependency versions change.

## Allocation control correction

The first four-process run at `1f581a5` completes 800 measured pairs with exact
outputs and zero diagnostics. Cow queued/legacy median ratios are 0.763–0.809;
bird ratios are 0.954–1.003. However, that implementation supplies one lane with
cloned buffers and the other with original assembled buffers. Allocation capacity,
layout and cache history are not necessarily equal, despite identical bytes.
Those retained results are provisional, not accepted scheduler evidence.

The corrected probe clones both lanes and balances clone order against execution
order. Its fresh results belong in the `balanced/` subdirectory of the original
evidence directory. The declared thresholds and whole-application limitations
remain unchanged.
