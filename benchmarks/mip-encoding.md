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
