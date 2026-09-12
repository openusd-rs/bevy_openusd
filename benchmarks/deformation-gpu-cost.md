# Showcase GPU-pass cost

Measured at b3c48bd on RTX 4080, Vulkan, NVIDIA 595.84, Bevy 0.19.1. Four
sequential release runs use order CPU, GPU, GPU, CPU. Each gathers 120 fresh
GPU timestamp measurements per pass after readiness warmup. No concurrent
build/test workload was intentionally launched during capture sampling.

```sh
# Set USD_CPU_SKINNING=1 for CPU controls; unset it for GPU runs.
USD_CAPTURE_GPU_SAMPLES=120 USD_CAPTURE_TIMEOUT_SECS=180 \
USD_CAPTURE_DOME=/Showcase/Environment USD_CAPTURE_RENDERER=forward \
USD_CAPTURE_SHADOWS=off make run WAYLAND_DISPLAY=wayland-0 \
  CARGO='cargo --offline' APP_TARGET='--release --example viewer_capture' \
  ARGS='assets/flagship_showcase.usda target/NEW.png 30'
```

The camera is (6,4,8) looking at (0,1,0), 1280x720. The animation is held at
time 30: this measures repeatedly rendering a posed scene, not advancing its
clock or uploading a new CPU-deformed mesh each frame. GPU runs report one
skinned and one morphed visible entity; CPU controls report neither. Both use
the same filtered dome, with one recorded generation and no active generator
at capture. Cold dome-filtering cost is outside the sampling window.

| Run | Opaque median ms | Opaque p95 ms | Opaque max ms | Transparent median ms |
|---|---:|---:|---:|---:|
| 0 CPU | 0.027648 | 0.028672 | 0.029696 | 0.016384 |
| 1 GPU | 0.029696 | 0.030720 | 0.031744 | 0.016384 |
| 2 GPU | 0.028672 | 0.030720 | 0.031744 | 0.016384 |
| 3 CPU | 0.027648 | 0.028672 | 0.029696 | 0.016384 |

GPU deformation adds approximately 0.001–0.002ms to the observed opaque-pass
median in this small scene. This is not a total-frame-time measurement or an
overall speedup claim. GPU clocks are uncontrolled; four runs on one device
do not establish scaling, cross-device behavior or sustained playback cost.
Do not sum per-pass percentiles into a frame percentile.

Both matched image comparisons pass RGB tolerance one across 921600 pixels,
with max RGB difference one. The first CPU/GPU pair was visually inspected:
the posed meshes and directional dome contribution on the sphere are present.
Artifacts: `target/showcase-gpu-cost-{0-cpu,1-gpu,2-gpu,3-cpu}.*`; every sidecar
retains raw samples and all pass statistics. Logs use matching `/tmp/` names;
comparison logs are `/tmp/showcase-gpu-cost-{first,second}-compare.log`.
