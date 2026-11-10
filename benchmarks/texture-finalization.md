# Texture-finalization optimization

## Scope

This step reduces CPU mip generation time and temporary memory without changing
resolution, filter policy, color space, sampling, quality settings or dependencies.
It does not solve animation preparation, native frame pacing or all loading stalls.
The complete objective remains in [`OPTIMIZATION.md`](../OPTIMIZATION.md).

`d709b52` adds opt-in `TextureSamplingTiming` counters and benchmark reporting.
Counters include mip construction, sampler assignment and generated-image
finalization. Raw per-seek deltas are emitted outside the timed `App::update`.
Profiling is disabled unless the resource is inserted; `USD_PROFILE_ROUTES=1`
inserts it in `editor_benchmark`.

## Diagnosis

Evidence: `/tmp/bevy-optimization-texture-baseline.1sCvsn/` and
`/tmp/bevy-optimization-cow-texture-before.log`.

- Cow opening spends about 1.97–1.99 s finalizing five textures.
- Its first measured seek takes about 2.91 s, including 1.57 s finalizing four
  textures. Later seeks finalize none.
- Bird opening spends about 3.54 s finalizing five textures, with none rebuilt
  during measured animation.
- `USD_PROFILE_LOADING=1` also reports a cow reload of 84 watched files, with
  five classified as changed, about 0.414 s inside the reload command. The bird's
  corresponding verification reports no changed files.

Source inspection identifies a follow-up: initial `prepare_textures` reads through
`UsdSource::read_asset`, whose resolver has no `disk_baselines`. Stage layer reads
do record baselines. Thus separate texture files can lack initial provenance and
be treated as replacements by the first reload verification. This still needs a
focused reproducer and repair preserving edit/reload conflict and snapshot safety;
do not suppress the watcher or merely extend its interval.

## Algorithm and fidelity: `e573343`

The first mip reads and decodes the original byte buffer directly into the reduced
float level. It no longer allocates a full-resolution float copy. Later levels
still average the unquantized previous float level, not the encoded bytes. The
filter order, odd-dimension handling and full chain to 1x1 remain unchanged.

A 256-entry sRGB-to-linear table uses the same float formula as before; alpha
remains linear. This removes repeated power evaluations for identical 8-bit
codes. Encoding is unchanged. Zero-sized or overflowing malformed dimensions
return without modifying the image.

Tests compare all bytes and mip counts against an independent full-float scalar
reference for five supported formats across nine square, odd and narrow sizes,
including finite half-float HDR values. All 256 sRGB codes and alpha values are
checked bitwise. Repeated finalization, unsupported/malformed images and optional
timing-resource behavior are covered.

An initial arbitrary-half-bit comparison found differences only in NaN payloads
when floating-point evaluation changed. The final tests distinguish the contract:
finite mip values are byte-exact; nonfinite half values retain their value/class,
with NaN payload/sign not required to be stable. Original base-level texels remain
byte-exact even when they contain NaNs. No finite-image tolerance was relaxed.

## Alternating release measurements

Evidence: `/tmp/bevy-optimization-mips-abba.GcocBJ/`, including before/after binary
hashes, exact source diff, raw samples and host-load records. Same Rust 1.98.1
release profile/default features; order before, after, after, before, one fresh
App per process. Each case uses four warmup and 100 measured repeated seeks.
No other task-owned build or benchmark overlaps; unrelated host applications
remain active. All 12 processes succeeded.

| Measurement | Before range | After range |
| --- | ---: | ---: |
| Bird CPU open | 4.893–4.927 s | 2.900–2.925 s |
| Bird initial texture finalization | 3.538–3.553 s | 1.550–1.552 s |
| Bird process peak RSS | 1,172.5 MB | 976.8–977.1 MB |
| Cow CPU open | 2.541–2.583 s | 1.493–1.551 s |
| Cow initial texture finalization | 1.951–1.985 s | 0.913–0.957 s |
| Cow worst measured seek | 2.873–2.912 s | 1.934–1.968 s |
| Cow seek texture finalization, cumulative | 1.545–1.579 s | 0.602–0.604 s |
| Cow process peak RSS | 1,059.1–1,081.1 MB | 817.9 MB |

MB is decimal. Peak RSS is Linux VmHWM for a fresh process, not VRAM or retained
allocation. Plugin startup is excluded from CPU-open timing but included in RSS.
Retained image/mesh payload, entity counts and peak asset counts match before and
after in every case. This change saves temporary memory, not retained texture
payload. The unchanged small morph control is noisy (roughly 0.295–0.340 ms median)
and finalizes no textures during seeks; no animation speedup is claimed.

Bird and cow steady CPU updates remain around 32–36 ms and 78 ms respectively in
this quieter run. Those numbers are not displayed FPS, and neither meets the
60 Hz target. The first cow reload remains a separate, unresolved stall.

## Reproduction and verification

```sh
export CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
USD_PROFILE_LOADING=1 USD_PROFILE_ROUTES=1 USD_BENCH_RAW_SAMPLES=1 \
  nix develop --command oslo make run-example --profile release \
  --name editor_benchmark \
  --args 'assets/external/cows/cow_eating.usda 1 gpu-prepared seek'
```

`test-all` passes: 874 tests, 20 ignored, no failures. Log:
`/tmp/bevy-optimization-mips-tests2.log`.

Before/after Vulkan captures at 1280x720 with 4x MSAA and validation enabled are
pixel-exact at RGB tolerance zero for hummingbird at 0, cow at 5 and the clearcoat
fixture at 0. Visible mesh counts and payload match. Images were inspected.
Evidence: `/tmp/bevy-optimization-mips-captures.hSIPcL/`. Both cow runs retain the
existing unavailable `OmniPBR.mdl` warning, so the combined capture gate correctly
returns failure for that diagnostic, not for an image difference.

`harden` passes: 787 no-default tests (21 ignored), 880 all-feature/all-target tests
(29 ignored), strict Clippy, formatting and whitespace checks. Log:
`/tmp/bevy-optimization-mips-harden2.log`. The preceding hardening attempt caught
three fixed-size slice-loop Clippy errors in the new tests; `e6da959` uses array
chunks without changing production code.

`test-watchers` also passes all five native asset-watcher and four editor texture
watcher tests. `check-web` passes wasm32 compilation. Logs:
`/tmp/bevy-optimization-mips-watchers.log` and `/tmp/bevy-optimization-mips-web.log`.
This is not a browser-rendering or browser-performance claim. The previously
documented strict real-bird CPU/GPU mismatch remains open.

All work stays local in focused commits. The user-controlled viewer is left open
at the earlier accepted freeze-repair checkpoint, not silently relaunched.

## Next investigation

The initial-reload repair and its verification are now recorded in
[initial texture verification](texture-reload.md). The following items describe
the investigation that followed this mip-generation checkpoint.

1. Reproduce the initial-reload issue with an external multi-texel texture, an
   editor open and a no-change `ReloadSources` command. Check recorded baselines,
   texture handle stability and finalization counters; a 1x1 fixture can miss
   regeneration because it has no appended mip data.
2. Preserve provenance for the exact encoded bytes used during decoding, including
   package outer-file identity and replacement snapshots. Inspect
   `source.rs::read_asset`, `SourceResolver::open_asset`, `editor.rs::prepare_textures`
   and `editor/reload.rs::changed_file`. Do not establish a baseline from a later
   disk read that could conceal a real edit made during loading.
3. Re-run genuine file-change, missing-file recovery, undo/dirty-layer protection,
   source snapshot and native watcher checks, then repeat the cow seek measurement.
