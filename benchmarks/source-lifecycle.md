# Release CPU baseline

Measured checkout `40a13c9e5eec0f06c025bad35a6a94d9f937a724`, Bevy 0.19.1,
vendored OpenUSD based on b7df5add with the recorded local patches. The clean
Mara path dependency was at `da74181bf49fd4ef86ab1b1f5c57adf932825710`.
Toolchain: rustc 1.96.0, LLVM 22.1.2, x86_64-unknown-linux-gnu; default Cargo
release profile. Host: i7-13700KF, 24 logical CPUs, Linux 7.0.0-30-generic.
One-minute system load ranged from 21.78 to 27.50 during build/measurement;
CPU affinity, background work and frequency were not controlled. Three samples
are descriptive, not statistical confidence intervals or regression thresholds.

## Commands and scope

```sh
TMPDIR="$PWD/target/test-tmp" make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--release --example source_benchmark' ARGS=128
TMPDIR="$PWD/target/test-tmp" make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--release --example source_benchmark' ARGS=1024
USD_PROFILE_SOURCES=1 USD_PROFILE_ROUTES=1 TMPDIR="$PWD/target/test-tmp" make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--release --example source_benchmark' ARGS=128
TMPDIR="$PWD/target/test-tmp" make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--release --example projection_benchmark' ARGS=128
```

All four commands exited successfully. Compilation is outside the reported
timings. These are headless CPU measurements: no disk loading, GPU upload,
rendering, screenshot acceptance or native USD renderer comparison. The source
benchmark also excludes plugin startup and input construction. The one/four-root
order alternates between samples; the first measured load may still include lazy
initialization. Timings from different fixtures must not be subtracted to infer
the cost of a particular subsystem.

## Source-root lifecycle

The fixture captures an external model and references it through native USD
instances. It verifies Ready roots, distinct projected entities, one initial
mesh/material asset, shared updated mesh handles, updated cube dimensions and
retained entity identity/runtime-only components after dependency replacement.
Idle values are means over 100 App updates. Asset sharing does not prove avoided
tessellation, reduced draw calls or low resident memory.

Raw uninstrumented samples:

```csv
instances_per_root,roots,sample,load_ms,reload_ms,idle_us,initial_meshes,initial_materials,projected_meshes
128,1,0,21.370,36.025,82.414,1,1,128
128,4,0,111.746,110.974,50.948,1,1,512
128,4,1,56.960,67.673,46.236,1,1,512
128,1,1,21.930,29.253,52.295,1,1,128
128,1,2,21.838,29.212,26.372,1,1,128
128,4,2,63.397,122.524,103.297,1,1,512
1024,1,0,107.942,148.831,58.797,1,1,1024
1024,4,0,560.289,870.955,76.101,1,1,4096
1024,4,1,662.067,600.009,78.130,1,1,4096
1024,1,1,115.566,175.773,42.647,1,1,1024
1024,1,2,171.053,201.584,21.852,1,1,1024
1024,4,2,424.978,596.079,62.384,1,1,4096
```

Four roots with 1,024 instances each take 596.079–870.955 ms per synchronous
dependency reload in this run. Low idle cost does not remove that update stall.
The smaller four-root fixture takes 67.673–122.524 ms. This establishes a release
baseline, not that the latest opacity-read change caused an improvement: no
matched release run of the preceding implementation was measured.

Separate instrumented four-root, 128-instance reload samples, all in milliseconds:

| Sample | Open | Overrides | Validation | Projection | Shapes apply | Material apply | Visibility apply |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | 1.792 | 0.001 | 37.388 | 92.040 | 13.279 | 7.392 | 5.792 |
| 1 | 1.342 | 0.001 | 23.814 | 53.837 | 7.435 | 4.031 | 3.461 |
| 2 | 1.796 | 0.001 | 37.780 | 84.055 | 11.753 | 6.036 | 5.625 |

Route application times are nested within projection, not additional phases.
Each reload visits ShapesRoute 1,024 times with 512 matches; MaterialRoute and
VisibilityRoute each match 1,024 times. Projection and validation dominate stage
opening. Significant projection time remains outside these three callbacks;
optimizing only shape material reads cannot eliminate the publication stall.

## Direct projection cache comparison

This different fixture uses LiveStage directly, an internal prototype and 128
instances, including the visible prototype (129 meshes without interning).
It measures initial projection, an authored prototype size edit and mean idle
change processing over 1,000 calls, not a full Bevy App update. Nanosecond-scale
idle values are not application frame-time estimates.

```csv
cached,sample,open_ms,project_ms,edit_ms,idle_us,initial_meshes,initial_materials,cached_payload_bytes_after_edit
false,0,5.323,17.508,11.234,0.002,129,129,0
true,0,0.248,19.112,11.114,0.001,1,1,1824
true,1,0.236,13.792,12.302,0.003,1,1,1824
false,1,0.316,12.132,11.533,0.002,129,129,0
false,2,0.321,12.541,9.228,0.002,129,129,0
true,2,0.288,14.329,10.871,0.002,1,1,1824
```

Cached projection was slower in every same-index sample; edit timings overlap.
The established benefit here is asset-count reduction, not CPU acceleration.
The 1,824-byte value counts retained mesh payload only, not process or GPU memory.
Source-root publication, direct edits and rendered performance remain separate
acceptance requirements. Next profiling should isolate work outside route callbacks
and test representative mesh/material scenes before introducing further caches.
