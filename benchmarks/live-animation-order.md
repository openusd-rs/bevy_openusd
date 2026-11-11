# Live animation traversal order

## Evidence and scope

The [image-capacity matrix](mip-capacity.md#matched-real-scene-capacity-measurement)
records different retained tangent-cache payloads between fresh executions of
the same baseline. Live resampling passes `AnimatedPrims(HashSet<String>)`
iteration directly to built-in animation batching. That order is not stable
across independently seeded sets. The path/entity map is already a BTreeMap,
and worker results already publish in input order; neither sorts this input.
Tangent admission and recency depend on visitation order.

Investigate a path-ordered view for built-in live animation only. Preserve the
public HashSet resource, custom callback order, route registration order,
same-clock short circuit, membership snapshots, changes made by custom callbacks,
source edits/reloads and independently timed asset roots. Keep current projection,
parallel-job and cache budgets. Do not introduce persistent ordering caches or
retain stage/entity history. Sorting scratch holds references for the current
pass only; the existing owned animated-set snapshot remains unchanged.

## Acceptance declared before implementation

- Reproduce unordered built-in traversal. Verify lexical order independently of
  hash seed/insertion order, empty sets, and externally changed membership.
- Exercise actual custom callbacks with and without the built-in registry.
  Preserve their current iteration order and resource visibility; mutations to
  animated membership during a callback affect the next time change, not the
  current snapshot. Repeating a clock must not run callbacks again.
- Pass hardening, native deformation, watcher/export and wasm checks. Keep the
  public `AnimatedPrims` and `SchemaRegistry` APIs intact.
- Use saved release CPU editor binaries in matched fresh-process ABBA runs for
  bird/cow/arm/subsets, idle and quick seeks, with the same allocation recorder.
  Candidate tangent footprints must repeat within each workload and remain
  inside existing bounds. Payload, geometry and asset counts must be preserved.
  Do not require the newly ordered cache to retain the same entries as a randomly
  ordered baseline; retain the earlier equality failures as reproduction evidence.
  Record all timing observations and flag greater-than-5% pair-mean increases.
- Validate rendered output through the **live-editor resampling path** after
  actual time changes. Ordinary `viewer_capture` uses asset roots, so its static
  images cannot establish this change's live-route fidelity. Do not automate the
  user's desktop or Play button; use owned application/offscreen capture.

Stable order is a hypothesis about the observed cache variation until tested in
fresh processes. It does not by itself establish faster playback or complete the
loading, memory, browser, native-window or 60 Hz acceptance requirements.

## Reproducer and implementation

The pre-change library run passes 728 tests and fails only the new ordering
fixture: it visits `/b, /A/Child, /Z, /B, /A` rather than lexical path order.
Both actual custom-registry cases already pass, including a callback adding
`/Late` to the public animated set while the current snapshot is being visited.
Log: `/tmp/bevy-optimization-animation-order-before.log`.

The candidate sorts borrowed path references only when `is_builtin_only()` is
true. Custom registries retain their original HashSet iterator without creating
the sorting vector. The existing cloned membership snapshot and lazy path/entity
filter stay in place. Tests cover independently constructed sets, changed
membership, empty sets, custom-only and extended-built-in registries, callback
visibility/mutation of the animated resource, restoration of the live stage/map,
and suppression of repeated-clock callbacks.

Workspace hardening passes 841 no-default / 958 all-feature/all-target test
executions, strict Clippy and formatting. Both new tests pass in both feature
configurations. Log: `/tmp/bevy-optimization-animation-order-harden.log`.
Native deformation (4), watcher cases (5 + 4), native exports (14) and custom
round trip (1) pass, as does wasm compilation. The gate ledger is
`/tmp/bevy-optimization-animation-order-gates.tsv`; corresponding recipe logs
use the `/tmp/bevy-optimization-animation-order-` prefix.

## Fresh-process comparison

Candidate `8d22fec` and the preceding capacity/recorder baseline run through the
same executable path, with no task-owned compilation during measurement. The
matrix covers bird/cow/arm/subsets, idle and quick seeks, in ABBA order: 32 fresh
processes and 192 raw warmup/measured updates. Both versions use the same recorder,
release profile, GPU-prepared CPU editor, profile flags and input assets. This
does not create a GPU device or measure native-window cadence. There are no
process warmups; OS page-cache state is uncontrolled. Observed host load varies
from 8.07 to 28.09.

Artifacts: `/tmp/bevy-optimization-animation-order-abba.3nlrQR/`, including logs,
raw samples, manifest checks, ledger and `analysis.json`. Binary SHA-256:

- Before: `7bcda02ba5fd391092a9ddf90eebf1d30385aef86f6475171acf0261c0634162`
- After: `b51bd9e678a4d29abd1546ca6912534ad44c18346e7f908651f221f1431268d2`

All processes, log checks and source/asset/binary manifest checks pass. Geometry,
image payload/capacity, peak asset counts and non-tangent cache payloads match.
The ordered candidate repeats its tangent payload within every workload. Quick
seek retained bytes are:

| Scene | Baseline executions | Ordered executions |
| --- | ---: | ---: |
| Bird | 2,738,736 / 3,296,424 | 2,742,864 / 2,742,864 |
| Cow | 15,574,944 / 16,572,456 | 15,808,584 / 15,808,584 |

All remain below the unchanged 16 MiB tangent-cache limit. Idle payloads repeat
for both versions; arm/subsets retain zero tangent bytes. This supports the
ordering hypothesis for these workloads, not determinism for every scene or
custom registry.

The overall matrix still **fails three predeclared timing checks**: cow quick
idle +400.143%, arm quick opening +30.876%, and subsets quick opening +26.192%
(pair means). Candidate cow seek medians range from 34.637 to 90.965 ms; even
lower pair means do not establish a causal playback improvement or 60 Hz.
All observed timing changes, RSS values and earlier failures remain recorded.
There are zero semantic-invariant, candidate-repeatability or diagnostic
failures; this does not convert the timing failures into passes.

## Live-editor capture harness

The rendered check uses an opt-in screenshot seek sequence in the embedded
viewer: bounded finite time codes, real
`EditorBridge` seek commands on separate updates, observed-clock acknowledgement,
document-change reset, and final-clock settling through the existing capture
gate and timeout. Preserve ordinary screenshot behavior when not requested.
Record requested and observed clocks alongside the final image metadata. Use
owned isolated Weston instances, not desktop input or UI replay. Both saved
before/after binaries must contain identical capture instrumentation; compare
the viewport after forward, reverse and repeated seeks, including a CPU control.

The embedded screenshot harness now accepts `USD_CAPTURE_SEEKS=5,10,5,5` only
when `USD_SCREENSHOT` is configured. It accepts 1..32 finite comma-separated
clocks, issues one command per update, waits for each clock on a subsequent
update, resets on document replacement and requires the final clock to remain
current for the existing 120-update/delay gate. The existing 60-second deadline
also covers unacknowledged commands. Metadata includes requested/observed clocks
and the initial time; ordinary screenshot requests still need no sequence.
Four tests cover bounds, same-clock/reverse seeks, acknowledgement, replacement,
settling/timeout and actual `EditorPlugin`/`LiveStagePlugin` animated transforms.
Hardening passes 845 no-default / 962 all-feature/all-target executions, strict
Clippy and formatting. The first hardening attempt caught a collapsible nested
conditional; it was corrected before the passing final run. Logs:
`/tmp/bevy-optimization-live-seek-capture-{harden,final-harden}.log`.
The harness tests alone are not rendered-image results.

## Rendered live-editor comparison

The matched viewer matrix is retained at
`/tmp/bevy-optimization-live-order-captures.1WEBem/`. It contains 20 isolated
viewer runs, ten before/after pairs, requested/observed clock metadata, tightly
packed RGBA and PNG viewport captures, compositor frames, all logs, manifests,
comparison output, and `analysis.json`. Saved binaries share capture code
`72a2d73`; the unordered baseline disables only sorting in `animation_paths`, as
recorded in `baseline-only.patch`. Source restoration is byte-checked and the
restored candidate rebuild matches the saved executable exactly. SHA-256:

- Before: `ba96c67fc6689f9210286891ec98d88347626a30f5eb2473a7e757099abc8677`
- After: `5cf559c616fd160245d11d298eff94cd662a5c9d5ca080c994cee2755fdd1394`

`oslo make build --profile release` builds the viewer without launching it;
`d1c31d1` adds that option while retaining the debug default. Invalid profiles
are rejected. Capture tools are built separately, and all captures use saved
binaries through `run-headless-benchmark`, with no task-owned compilation.
Wasm compilation passes again after the capture change.

Each viewer runs in an owned headless Weston instance with a private runtime
directory/bus, without user-desktop input or UI replay. Weston records the
RTX 4080 Vulkan device; viewport readbacks are 1440 x 920. Four GPU-skinning-mode
scenes use `[5, 10, 5, 5]` and `[5, 0, 10, 10]`, starting at zero. Bird and cow
also have explicit CPU-skinning controls for the first sequence. All requested
clocks are observed on subsequent editor updates; final captures satisfy the
120-update and one-second minimum settling gates. All process, image-region,
panic/render-error log, asset/source/binary manifest and semantic metadata checks
pass. Camera matrices, mesh residency and capture dimensions match within pairs.

Seven pairs are pixel-exact at RGB tolerance zero: cow (both GPU clocks and CPU),
arm (both clocks), and subsets (both clocks). **Three bird pairs fail**:

| Mode / final clock | Changed pixels out of 1,324,800 | Maximum RGB error |
| --- | ---: | ---: |
| GPU / 5 | 203 | 2 |
| GPU / 10 | 205 | 2 |
| CPU / 5 | 212 | 2 |

The GPU-five difference image was inspected: changes visually align with the
background grid rather than an obvious bird-shape change. Its changed-pixel
bounds span `[2, 175, 1427, 910]`. This is a diagnostic observation, not a proven
cause or a reason to exclude those pixels. One additional fresh run of **each
same saved binary** reproduces its own GPU-five image exactly (zero changed
pixels), so these controls do not support dismissing the cross-binary failure
as random run-to-run variation. Their logs, metadata, comparisons and
`repeat-results.tsv` remain alongside the original matrix.

Both versions show changed rendered poses between clocks five and ten: bird
75,876 pixels, cow 105,420 and arm 25,944. Bird poses, cow and arm images were
visually inspected. These images prove actual live seek rendering, not playback
cadence or the existing strict CPU/GPU oracle's acceptance.

The broad zero-warning diagnostic also fails in every viewer run (4 or 5
records each): the headless clipboard lacks data-control/X11 support, the Vulkan
loader skips an unavailable D3D12 ICD, and some private-bus appearance requests
time out. Logs retain these failures; none is silently filtered into a pass.
The private bus also auto-activates system portal backends from inherited desktop
selection, so future capture isolation should explicitly select a headless/GTK
portal configuration rather than inherit that selection. No desktop-control or
input commands were issued. These embedded captures do not resolve the earlier
asset-root cow MDL diagnostic.

Overall acceptance remains failed: three strict image comparisons and twenty
runtime-diagnostic rows in the main matrix, in addition to the CPU timing
failures above. Next: compare unseeked controls and inspect grid transform/settings
and extracted camera state before attributing the repeatable grid difference to
animation traversal. Preserve full-frame zero-tolerance checks and all original
failures. Native cadence, responsive loading, browser and lifecycle work remain
open under `OPTIMIZATION.md`.

### Unseeked controls and render-state probe

Four saved-viewer controls in
`/tmp/bevy-optimization-live-order-unseeked.KtpHpt/` open directly at time zero
or five with no seek commands. Both before/after pairs are pixel-exact at
tolerance zero; metadata matches except elapsed settling time and confirms empty
requested/observed sequences. All four capture processes succeed. Headless
loader/clipboard diagnostics remain (4 or 5 records per run), so the combined
control driver still fails. These results distinguish opening at a pose from
the failing live-seek sequences; they do not establish the cause of the latter.

The controls explicitly select Weston/GTK portal configuration inside their
owned private runtime rather than inheriting the user's desktop selection.
None activates Hyprland or KDE portal backends. The reusable capture-session
helper now applies that private-only environment; its regression checks that
the caller's desktop, display and XDG environment remain unchanged.

The existing transparent-phase probe is shared from `src/transparent_probe.rs`
by both the offscreen example and embedded screenshot harness. Setting
`USD_CAPTURE_PHASE_PROBE` on a screenshot request records actual sorted phase
items, extracted grid transforms/settings, and extracted camera matrices,
viewport, render format and jitter. Its scope remains the latest completed
render phase after sorting and before batching/preparation, not a claim about
GPU uniform readback or the screenshot's exact submission. The normal viewer
does not register this opt-in probe. Hardening passes 848 no-default / 967
all-feature/all-target executions, formatting and strict Clippy; regression
coverage includes display-setting changes and optional extracted projection
overrides/jitter. Log: `/tmp/bevy-optimization-live-order-probe-harden.log`.
