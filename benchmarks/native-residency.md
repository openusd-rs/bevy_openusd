# Native residency reveal verification

## Contract declared before rendering

The viewer's experimental `USD_DEFER_HIDDEN_MESHES=1` option enables the existing
library resource. Unset or `0` remains eager; other values fail validation. The
library default is unchanged. This exposes the policy tested in
[the paired CPU comparison](viewer-residency.md), not an accepted default switch.

`USD_CAPTURE_RESIDENCY` is an optional JSON capture scenario, used only with
`USD_SCREENSHOT`. For `assets/residency_reveal.usda`, use
`{"prim":"/Test","mesh":"/Test/Face","steps":4}`. Steps select the initially
hidden mesh, reveal its parent, undo visibility and redo visibility. Prefixes
1..4 allow independent native readbacks of every state. Input is bounded to
8 KiB, unknown fields are rejected, and no save, Play or arbitrary command is
available. The fixture retains animated skin/morph normals and material subsets
and adds a visible blue backdrop for stable initial framing.

After the existing seek sequence, wait 60 updates before issuing the first
command. Acknowledge each command from the actual editor snapshot and projected
mesh before issuing the next. Require a pristine hidden initial state, preserved
document/clock/entity/camera, selected mesh identity, exact revision/history and
authored/inherited visibility transitions. Deferred geometry stays absent while
selected and hidden, becomes resident on reveal, and stays resident after undo.
After the final acknowledgement, the ordinary 120-update/minimum-delay capture
gate still applies. Record every observed state and the initial camera in the
readback metadata. Failed commands or unexpected state changes fail the capture.

Use one saved release viewer at a common executable path in owned isolated
Weston sessions, with private configuration/runtime/bus and no user-desktop or
input automation. Seek 0,10,7,7 from initial time 0, then capture prefixes 1..4
for eager/deferred policies with both CPU and GPU-skinning configurations.
Rendering quality, source assets, camera and resolution must match.

Require zero-RGB-tolerance comparisons for all eight eager/deferred pairs,
undo-vs-selected and redo-vs-reveal within each mode/policy (eight pairs), and
CPU-vs-GPU within each policy/prefix (eight pairs). Require visible reveal changes
in more than 100 pixels, inspect actual images, retain all diagnostics, and
verify manifests before/after. These are correctness checks, not native-window
cadence or before/after performance measurements. No task-owned build overlaps
rendering; unrelated work remains untouched.

Even a complete fixture pass does not clear earlier real-scene grid/CPU-GPU/MDL
failures, lifecycle/platform acceptance or the 60 Hz target. Library/viewer
defaults remain eager until the broader policy acceptance is established.

## Harness checkpoint

Hardening passes 859 no-default and 982 all-feature/all-target executions,
strict Clippy and formatting. Tests validate configuration bounds, wait for
command acknowledgements, reject document/clock/entity/policy/camera drift and
exercise all four real commands in CPU/GPU and eager/deferred apps with actual
transform/visibility propagation. The minimal headless fixture includes Bevy's
`MeshPlugin`, which supplies the inverse-bind-pose asset resource required by
visibility's skinned-bounds system. The initial compile/fixture failures and
passing log are retained in `/tmp/bevy-optimization-native-residency.tjQ8R6/`;
the successful run is `harden-mesh-plugin.log`.

## Device-limit failure found by the native smoke

The initial deferred redo smoke reaches the real skin/morph pipeline and fails:
vertex-stage storage buffer count is 9 while the shared Mara device requested
only 8. Both PBR prepass and opaque pipelines report validation errors, and no
viewport readback completes. The compositor image alone is not a rendering
pass. Logs and the image remain under `smoke/`; the original saved viewer hash is
`af546d21ee258828aaec0c0123e3ba3b647f74906a9ba79c8670ef4cf5f46ea2`.

The viewer now uses Mara's existing `WindowApp::configure_gpu_limits` hook to
request up to 16 storage buffers per shader stage, bounded by adapter support
and preserving any higher existing host requirement and all other limits.
This changes device negotiation, not geometry, skin weights, shader quality or
validation. Native readback metadata records both actual device and adapter
limits. Hardening passes 860 / 983 executions after adding the hook regression.
The new rendering run must establish that the hardware request actually fixes
the pipeline failure; unit tests alone cannot do that.

Adapters offering fewer than the pipeline's required buffers are not claimed
fixed by this bounded request. Their capability-specific deformation fallback
and native acceptance remain outstanding. The isolated compositor also reports
portal/clipboard/unused-ICD warnings; these remain in the diagnostic ledger and
are not silently waived or suppressed.

## Native result: device failure fixed, image gate still fails

`00d9bd6` builds the corrected viewer. The repeated deferred redo smoke records
an actual device limit of 16 and adapter limit of 524,288; its viewport readback
completes without pipeline validation errors. The image was inspected. Corrected
viewer SHA-256: `ce1592dfff4b6bfb495ac75a17f77668489caef798feb822262aa0f744b73f73`.
The original failed smoke remains intact. This fixes the observed shared-device
negotiation failure on this hardware, not low-limit adapter support in general.

The subsequent matrix completes all 16 native processes and readbacks on RTX
4080/Vulkan, 1440x920, unchanged viewer quality. Every seek and command prefix is
acknowledged; selected identity, document, clock 7, mesh entity, camera matrices,
revision/history, authored/inherited visibility and residency transitions verify.
Source/executable/lockfile manifests verify before/after. No task-owned build
overlaps rendering; observed one-minute host load ranges from 2.91 to 8.65.

All 16 runtime-render log checks pass: no pipeline validation error or capture
failure remains. All 16 broader diagnostic checks still fail on isolated-host
clipboard/unused-ICD warnings, plus one portal timeout. These are not omitted.
Sixteen of the 24 exact RGB comparisons pass, and eight fail:

| Failed comparison | Changed pixels | Maximum RGB error |
| --- | ---: | ---: |
| GPU eager reveal vs deferred reveal | 39,764 | 182 |
| GPU deferred reveal vs redo | 39,764 | 182 |
| CPU eager reveal vs deferred reveal | 219 | 2 |
| CPU deferred reveal vs redo | 219 | 2 |
| Eager reveal, CPU vs GPU | 39,764 | 182 |
| Eager redo, CPU vs GPU | 39,764 | 182 |
| Deferred reveal, CPU vs GPU | 220 | 2 |
| Deferred redo, CPU vs GPU | 39,764 | 182 |

All hidden/undo images match exactly. Eager/deferred redo also matches within
each CPU/GPU configuration. However, the eager GPU reveal/redo and deferred GPU
redo show a visibly different mesh shape from the CPU rendering; the initial
deferred GPU reveal follows the CPU shape. This is not only the older tiny grid
discrepancy. The inspected revealed images have a light material on the blue
backdrop, as authored by the strongest normal-material layer. Reveal changes
40,782–46,240 pixels, so the test is not comparing two empty images.

These failures keep native policy acceptance open. Do not enable deferral by
default, relax tolerance, disable GPU morphs, or repeat this matrix unchanged.
First compare parent/subset mesh, weights and palettes at these exact command
transitions, then inspect the render-world morph descriptor and cached mesh
input indices. Bevy 0.20.1 source has `prepare_morph_descriptors` reading
`RenderMeshInstances` before `collect_meshes_for_gpu_building` publishes newly
visible instances; the builder stores a sentinel when a descriptor is absent.
That ordering is a lead, not a proven root cause. A renderer witness and a
targeted regression are needed before choosing an invalidation/scheduling fix.

Evidence is retained in `/tmp/bevy-optimization-native-residency.tjQ8R6/`:
`smoke/`, `smoke-fixed/`, 16 native PNG/RGBA/readback reports, process and
compositor logs, 24 comparison logs, `results.tsv`, `comparisons.tsv`,
`analysis.json`, `analysis.log`, manifests and the saved driver/analyzer. The
combined driver exits nonzero; this is not a passed native acceptance gate.
