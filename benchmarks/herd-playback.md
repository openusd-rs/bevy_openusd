# Herd playback

A stress scene references `assets/external/cows/cow_eating.usda` N×N times on a
2.5 m × 4.5 m grid (one visible animated cow per reference; the other LODs are
hidden and deferred). Measured offscreen with `viewer_capture`,
`USD_CAPTURE_PLAYBACK_SAMPLES=300` and `USD_DEFER_HIDDEN_MESHES=1`, release, on
the local RTX 4080. The runner caps playback at 60 Hz. GPU passes stay below
0.2 ms throughout; the main thread is the limit.

| Herd | Before | After |
| --- | --- | --- |
| 16 cows | 49.6 Hz, main 19.5 ms | 59.8 Hz, main 3.3 ms |
| 36 cows | 22.5 Hz | 59.2 Hz, main 11.8 ms (host load ~7) |
| 64 cows | 12.2 Hz, main 76.4 ms | 59.7 Hz, main 10.2 ms |

Per-frame user instructions for 64 cows, from `perf stat` differences between
150- and 450-frame runs, fell from about 517 M to 90 M.

## Changes

- `d8bb161`: instance roots validated their composition revision (a stage
  flush plus every layer identifier) on each of ~11,700 prim lookups per frame,
  about 45 ms. The revision is now checked once per tick and the animated-prim
  list is reused until it changes; custom registries keep per-prim
  classification. Hidden, deferred deformation-only meshes skip resampling.
- `2393c81`: posed sort centers were rebuilt from every vertex of every
  deformed draw; only draws the last frame queued in a transparent or
  transmissive phase need them now.
- `c15e062`: poses are cached per frame by the composed specs (layer, spec
  path, time offset and scale) of the skeleton and its animation rather than
  by stage path, so references to the same data decode the animation once.
  Copies with layer offsets or local overrides keep separate poses.

Herd, cow and bird frames match the preceding builds byte for byte.
