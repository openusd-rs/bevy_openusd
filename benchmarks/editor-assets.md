# Real-asset editor CPU and payload baseline

Measured after `4bc455a` with `examples/editor_benchmark.rs`. The fixture uses
the actual EditorCommand::Open path, including file reading, source validation,
texture decoding, projection, initial animation/purpose processing and editor
snapshot publication. Each sample creates a new App and warms its plugins before
Open. Path canonicalization and command enqueueing are outside the timer.
The benchmark fails a non-Ready open and checks projected index bounds and
unchanged document identity during idle updates.

Bevy AssetPlugin and init_asset tracking are enabled. Payloads are counted after
100 idle App updates, letting unused-handle cleanup run; idle_us is the mean of
those updates and can include early cleanup. Initial prototype/source meshes
retained by caches are included. Counts include all retained assets and projected
mesh entities, not only visible geometry. Attribute and index bytes count raw
vector payloads, excluding capacities, allocator overhead, scene data, transient
peak allocations, render extraction, GPU alignment and VRAM. Morph bytes count
the retained MorphAttributes slice; image bytes count decoded Image data.
Unreferenced vertices are stored vertices absent from that asset's index buffer;
this is not a proof that their associated USD data can simply be deleted.

Rendering, widgets, GPU uploads, skinning on the GPU and frame rate are not
measured. The headless editor uses CPU deformation. Disk cache, thread scheduling
and background work are uncontrolled; repeated runs are not cold-disk tests.

## Environment and commands

Rust 1.96.0, default optimized Cargo release profile, Bevy 0.19.1, i7-13700KF,
Linux 7.0.0-30-generic. Collection HEAD:
`fe83a4a95eb99c16264dbc846a785dd00b8d888f`; unrelated collection modifications
were present and left untouched. These commands use the named existing ANYmal
and Spot assets, not the modified Robotti/tractor files.

```sh
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--release --example editor_benchmark' ARGS='../usd_collection/oems/anymal_anybotics/anymal_b_simple_description/anymal.usdc'
make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--release --example editor_benchmark' ARGS='../usd_collection/oems/spot_boston_dynamics/spot_base_urdf/spot.usdc'
```

An optional second argument sets 1–10 samples (default 3). Both runs completed
successfully. This is a current baseline, not a measured speedup attributable to
the preceding index-only subset change.

## Raw samples

```csv
asset,sample,open_ms,idle_us,mesh_entities,subset_entities,mesh_assets,vertices,unreferenced_vertices,vertex_bytes,index_bytes,morph_bytes,image_bytes
anymal,0,991.452,32.324,296,231,196,22517138,22149662,720548416,8466480,0,10240000
anymal,1,666.836,28.050,296,231,225,24112160,23691700,771589120,9681528,0,10240000
anymal,2,657.447,26.032,296,231,193,22902395,22553852,732876640,8006196,0,10240000
spot,0,31.187,28.047,26,0,8,14101,0,451232,65460,0,0
spot,1,23.702,24.827,26,0,8,14101,0,451232,65460,0,0
spot,2,23.986,27.004,26,0,8,14101,0,451232,65460,0,0
```

ANYmal has 231 material-subset entities and retains 720,548,416–771,589,120 bytes
of vertex attributes; over 98% of its retained vertices are not referenced by
their asset's indices. Counts varied across independent samples; these are
observed settled snapshots, not deterministic memory guarantees. Spot has no
material subsets and retains 451,232 vertex bytes with no unreferenced vertices.

Subset preparation still clones complete vertex layouts into individual parts
and the remainder, while replacing only indices. The next memory change should
compact those layouts together with joint attributes and per-target morph data.
Compacting positions alone would break deformation indexing. This benchmark does
not establish that all measured unreferenced bytes can be removed safely, or
that the device-bounded mesh allocator is no longer needed.

The bundled material_subsets test verifies exact entity/asset counts, vertex and
index payloads, unreferenced-vertex counts and rejection of failed opens. The
final benchmark uses tracked assets; an earlier untracked prototype overcounted
assets waiting for handle cleanup and is not used in these results.

## After subset vertex compaction

Implementation committed as `cffdd62`. Same release benchmark and ANYmal asset:

```csv
sample,open_ms,idle_us,mesh_entities,subset_entities,mesh_assets,vertices,unreferenced_vertices,vertex_bytes,index_bytes,morph_bytes,image_bytes
0,679.503,27.484,296,231,109,624035,310045,19969120,7244184,0,10240000
1,616.179,29.505,296,231,109,624035,310045,19969120,7244184,0,10240000
2,625.352,27.192,296,231,109,624035,310045,19969120,7244184,0,10240000
```

Retained vertex payload falls from 720.5–771.6 MB to 19.97 MB, with unchanged
mesh/subset entity counts. Each subset retains only referenced vertices, with
the same remapping applied to every attribute and every morph target. Source
cache assets still retain 310,045 unreferenced vertices; source cloning also
still incurs transient allocations. These measurements do not prove a reduction
in peak RSS or VRAM, nor make the device-bounded allocator unnecessary. Timing
ranges overlap and this is not a controlled CPU speedup experiment.

Raw after-compaction log: `/tmp/subset-compact-anymal.log`. Empty compacted
assets subsequently became CPU-only to avoid uploading zero-vertex buffers;
that upload policy is outside this CPU payload benchmark.
