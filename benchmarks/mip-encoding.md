# Exact sRGB mip encoding

## Measured target and invariants

The native [opening diagnostic](opening-milestones.md#native-texture-finalization-attribution)
finds synchronous mip/sampler/fingerprint finalization consumes 51%/72% of
bird/cow request-to-Ready time. The previous decode lookup/full-resolution-float
buffer optimization is already present. Remaining sRGB output encoding calls
the transfer-function power for each generated RGB channel; alpha is linear.

Investigate a bounded exact quantizer, not an approximate transfer curve:

- Retain the existing scalar transfer/rounding formula as the reference.
- Derive 255 quantization-transition thresholds with that formula on the running
  platform. An additional sentinel represents values already encoding to 255.
- A 4097-byte table maps each 1/4096 linear-input bin to its starting code. The
  transfer curve spans at most one output transition per bin; verify this
  invariant at construction/tests rather than silently assuming it.
- Lookup the bin's code and compare the input to its next exact threshold. Keep
  all original float mip averages; only replace the final float-to-byte encoding.
- Retain scalar fallback for any platform/table failing construction checks.
  The fixed table retains no images, assets, stages, entities or historical data.

Preserve source texels, all finite mip bytes and level counts, odd/narrow shapes,
the full chain to 1x1, sRGB/linear-alpha semantics, filtering, sampler wraps,
generated-image tracking and reload behavior. Half-float and linear formats
remain unchanged. Existing nonfinite half value/class behavior remains intact.
The full-float reference must not call the optimized encoder.

## Acceptance before implementation

- Check every transition and neighboring float, every table-bin boundary,
  random finite inputs and special values against the independent scalar path.
- A release-only exhaustive check covers all representable positive `f32`
  inputs from the first nonzero output transition through 1. Values below the
  transition and saturation/nonfinite cases have explicit boundary tests.
- Existing independent full-mip tests continue to compare all bytes for five
  formats and square/odd/narrow images. Native bird/cow/arm/subset renders must
  remain pixel-exact at tolerance zero.
- Save the pre-change release capture with opening/texture counters. Compare
  fresh processes in before/after/after/before order without task-owned build
  overlap; retain raw intervals, source hashes and host load. Target at least
  10% lower texture-finalization pair mean and 5% lower request-to-Ready pair
  mean on both textured scenes, with no greater-than-5% control regression.
  Failures remain failures; do not rerun until a favorable result appears.
- Run workspace hardening, native deformation/reload/export and wasm checks.
  This is not a claim of bounded main-thread loading, first-visible latency,
  browser/window acceptance or completion of `OPTIMIZATION.md`.

## Candidate implementation and initial checks

`4940ee6` separates the full-float mip reference from `Kind::encode`; its
library checks pass 723 no-default / 731 all-feature tests. Log:
`/tmp/bevy-optimization-mip-encoder-reference.log`. The candidate replaces only
non-alpha sRGB output encoding. Tables occupy 5124 bytes before lazy-init/option
bookkeeping; initialization checks endpoint outputs, ordered transitions and
their neighboring float codes, plus the one-transition-per-bin invariant.
A rejected table leaves the old scalar encoder active.

Regular tests cover every transition with sixteen neighboring floats on each
side, every input-bin boundary, special values, 500,000 seeded raw/uniform
float samples, alpha behavior and malformed quantizers. The independent full-mip
tests remain byte-exact for all five formats and existing odd/narrow sizes.
Workspace hardening passes 837 no-default / 953 all-feature/all-target test
executions, strict Clippy and formatting. Log:
`/tmp/bevy-optimization-mip-encoding-fixed-harden.log`. These configurations
overlap. The initial run in `...-mip-encoding-harden.log` failed strict Clippy on
a constant assertion in the ignored release test; a direct debug-build panic
guard replaces it without suppressing the lint.

`oslo make test-texture-encoding` builds a release library test executable and
runs two ignored checks serially: the exhaustive float comparison and an
alternating scalar/candidate encoder-only wall-time diagnostic. The latter uses
1,048,576 deterministic four-texel linear averages, four passes per observation,
and two before/after/after/before blocks. Both paths use the same indirect-call
and black-box harness and must return identical checksums. Table initialization
and input generation are outside that kernel interval; native opening includes
initialization. Kernel results do not replace whole-mip/opening measurements.

Exhaustive release, native rendering/performance, reload/export/deformation and
wasm acceptance remain pending at this implementation checkpoint.

## Completed correctness gates

Candidate `05b3106` passes the release exhaustive comparison for **107,011,437**
representable inputs, from just below the first nonzero code transition through
1. The kernel check also passes, with identical checksum 613,326,880 for every
4,194,304-value observation. Log:
`/tmp/bevy-optimization-mip-encoding-exhaustive.log`. The fresh release test
feature set required a nine-minute build; the two tests themselves finish in
0.86 seconds. No task-owned compilation overlaps the kernel measurements.

| Encoder-only observation | Scalar, ms | Candidate, ms |
| --- | ---: | ---: |
| First ABBA block | 34.472 / 34.107 | 8.756 / 8.571 |
| Second ABBA block | 33.318 / 38.053 | 8.363 / 8.605 |

The average observation falls from 34.988 to 8.574 ms (75.5% lower, about 4.08x
throughput). This proves an encoder-only improvement in this harness, not a
whole-image or loading claim. All constants and threshold calculations use the
same running platform's scalar formula; the exhaustive result is local Linux
evidence, not proof of every platform's floating-point library behavior.

Native gates also pass: four deformation-oracle tests, five asset watchers plus
four editor texture watchers, fourteen native exports and the custom-data
reference round trip. Wasm compilation and the release viewer-capture build pass.
The status ledger is `/tmp/bevy-optimization-mip-encoding-gates.tsv`; logs are
`/tmp/bevy-optimization-mip-encoding-{test-native-deformation,test-watchers,test-native,check-web,build}.log`.
Wasm compilation is not rendered browser or exhaustive browser-math acceptance.

Native rendered output and end-to-end opening measurements are the next gate;
the kernel result does not waive earlier loading or rendering failures.

## Native opening comparison

`/tmp/bevy-optimization-mip-encoding-abba.J038vE/` retains sixteen fresh native
processes, before/after/after/before for bird, cow, arm and subsets. Both binaries
include the same opening and texture counters. Baseline production is `5e3b238`
(saved `20c43b2` binary; intervening changes are documentation); candidate is
`05b3106`. SHA-256 values:

- Before: `aa9e2e3f7041f0f9c94a0909fdd6de856bbc64785610cd508da783aec8260c57`.
- After: `6657879141e3eebad2eccd406f75621e8b569e3e15bca0ab2eb547c12cad5172`.

Dependencies, assets, fixed cameras/poses, 1280x720, forward/4x-MSAA, validation,
default affinity, unlimited projection and sixty-ready-frame settling match.
The device remains RTX 4080/Vulkan. No task-owned compilation overlaps; one-minute
host load spans 34.64–46.04. Raw intervals and source/binary hashes are retained.

| Case | Before texture finalization, ms | After texture finalization, ms | Change of pair means | Before request-to-Ready, ms | After request-to-Ready, ms | Change of pair means |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Bird | 3283.465 / 1740.631 | 2119.074 / 2423.468 | -9.6% | 6665.002 / 4319.232 | 4455.648 / 6230.575 | -2.7% |
| Cow | 921.752 / 2872.137 | 1580.312 / 1068.681 | -30.2% | 1640.272 / 4009.843 | 3269.686 / 2231.788 | -2.6% |
| Arm | 0 / 0 | 0 / 0 | not applicable | 242.139 / 158.988 | 96.923 / 88.225 | -53.8% |
| Subsets | 0 / 0 | 0 / 0 | not applicable | 26.008 / 17.846 | 54.713 / 40.263 | +116.6% |

**Full performance acceptance fails eight declared checks.** Bird misses the 10%
texture target (9.585% observed); both textured scenes miss 5% request-to-Ready.
Arm pipeline observation/readback regress 51.9%/22.0%; subset Ready/pipeline/
readback regress 116.6%/302.8%/80.6%. The unchanged bird source-publication phases
increase 33.8%, while cow's unprofiled residual increases 87.6%. Neither the
kernel result nor variable host load waives these failures or isolates the full
observed texture-time changes as causal gains.

All sixteen opening/phase records pass order/arithmetic/configuration checks.
Texture image counts, input/appended bytes and visible mesh counts match;
texture counters at Ready equal their later screenshot values in every run.
All twelve image comparisons against each case's first baseline are pixel-exact
at tolerance zero; representative candidate bird/cow images were inspected.
All processes, runtime-log checks and source manifests pass. Cow retains four
missing-MDL warnings; the diagnostic driver and performance analysis exit 1.
The old CPU/GPU pixel discrepancy and other failed gates remain unresolved.

## Binary-identity control

The arm/subset cases finalize no textures, so they never call the new encoder,
yet their render-startup observations change substantially. Before another
production optimization, run the same opening harness with byte-identical
executables under its before/after labels, then counterbalance version order if
needed. Record every result, not just favorable blocks. This tests startup/order
variation without silently waiving the current failed gates. More controlled
whole-image/real-asset measurements are needed beyond the encoder-only kernel;
bounded main-thread loading and window/browser acceptance are still open.

The byte-identical control in
`/tmp/bevy-optimization-identical-openings.k7TBLe/` uses the candidate SHA above
for both labels. Eight fresh processes cover arm/subsets in ABBA order, with the
same settings and no task-owned compilation. All process/log/manifest checks
pass; six image pairs are pixel-exact. Texture-finalization counters remain zero.
Nevertheless, the `after` label's pipeline-observation pair mean is 64.5% higher
for arm and 162.5% higher for subsets; readback rises 27.2%/37.6%. These differences
cannot be attributed to a code change. Host load spans 33.18–41.08.

Own-process file traces, run in reversed after/before order, show each executable
opens a different NVIDIA driver-cache pair under the same GLCache directory.
The `before` label opens `5428040e00f1c7b4.bin` (819,029 bytes) and its 2,752-byte
index; `after` opens `24aea9e6f6f0e813.bin` and index, both 32 bytes. Both processes
read `/proc/self/exe`; executable contents remain identical. Trace process/log/
image checks pass. These are observations of the installed driver, not a general
claim about its cache-key algorithm or an explanation for the empty cache.
Traced timings are not performance-acceptance measurements. No host cache was
removed or manually edited, and no desktop/security settings were changed.

A second control, `/tmp/bevy-optimization-common-path-openings.j8AGBk/`, stages
each label at the same owned `run/viewer_capture` path, waits for completion
before replacing it, and performs one explicit unmeasured warmup per case.
All eight measured processes and image comparisons against their case's warmup
pass; every opening record validates. Arm pipeline means now differ by +1.0%,
while subset pipeline means differ by -48.6%. Thus the large consistent
`after`-label penalty is absent, but substantial variation remains even with
identical code. Host load is 65.25–85.03, higher than the first control; changing
both path and warmup does not isolate their individual effects. This is warm-cache
diagnostic evidence, not cold-start or full performance acceptance.

The next matched baseline/candidate run keeps the original thresholds and asset/
renderer settings, uses one common executable path for every measured process,
and retains two unmeasured warmups per case, one per binary, before measured ABBA.
Snapshots and assets are hashed before/after; all raw outputs, failures and
warnings are retained. The script is `/tmp/bevy-optimization-mip-common-path.sh`.
Earlier failed gates remain recorded rather than being retroactively passed.
