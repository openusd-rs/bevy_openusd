# Viewer logging I/O

## Change and boundaries

`src/main.rs::init_tracing` previously enabled TRACE for `usdview` and
`usd_bevy`. Its file writer opens, appends and closes `/tmp/usdview.log` for
each formatted event, in addition to writing stderr. Mesh attachment emits
one TRACE event per routed mesh. The separate headless editor benchmark does
not install this viewer subscriber.

The viewer now defaults to `warn,usdview=info,usd_bevy=info,openusd=info`.
Explicit `RUST_LOG` filters still replace that default; invalid or absent
filters still use the fallback. File handling, stderr, panic reporting,
library-consumer logging and rendering defaults are unchanged. This is a
measured reduction in log traffic, not an established playback bottleneck or
frame-rate improvement.

## Measurement

Evidence is local at `/tmp/bevy-optimization-viewer-log-io.WFLkgb/`.
`driver.sh` uses the saved release viewer containing scoped query sharing,
with the old default and an explicit INFO override. It runs bird TRACE/INFO,
then cow INFO/TRACE. `candidate-driver.sh` verifies the newly built viewer
with unset `RUST_LOG` and an explicit old TRACE filter on the bird.

Each process runs in a private Weston/DBus session, with eager residency,
default rendering quality, WGPU validation/debug enabled and a 1440 by 920
native viewport. EditorBridge issues 30 evenly spaced clocks from zero to
the authored end, followed by the midpoint twice: 32 seeks, including reverse
and repeated clocks. Bird ends at 132 after visiting 264; cow ends at 144
after visiting 288. Capture waits at least 120 ready updates and 1,000 ms.
This uses no user-desktop input or Play automation. No task-owned build
overlaps the captures; unrelated host work is left alone.

`strace --seccomp-bpf -f -qq -c -P /tmp/usdview.log` counts only
`openat,openat2,write,writev,close` for that application log. Intrusive tracing
times are not native frame time, total logging cost, or FPS. Counts cover
startup, seeks and capture readiness, not only an animation interval.

| Viewer / asset / filter | Log bytes | TRACE events | Writes | Opens | Closes | Total calls |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Saved / bird / TRACE | 233,118 | 1,271 | 1,280 | 1,281 | 1,281 | 3,842 |
| Saved / bird / INFO | 1,393 | 0 | 9 | 10 | 10 | 29 |
| Saved / cow / TRACE | 52,547 | 434 | 443 | 444 | 444 | 1,331 |
| Saved / cow / INFO | 1,397 | 0 | 9 | 10 | 10 | 29 |
| Candidate / bird / default | 1,393 | 0 | 9 | 10 | 10 | 29 |
| Candidate / bird / explicit TRACE | 233,118 | 1,271 | 1,280 | 1,281 | 1,281 | 3,842 |

All six runs retain five INFO and four WARN events. Normalizing timestamps
and instance addresses produces identical non-TRACE event multisets within
each asset. There are no syscall errors; each event has one write/open/close,
plus the initial truncate open/close. The 1,271 and 434 TRACE counts match
41 and 14 mesh attachments across 31 changed samples. INFO suppresses these
events without discarding the observed diagnostics.

## Verification and retained failures

Workspace hardening passes formatting, strict Clippy, 884 no-default and
1,011 all-feature/all-target test executions. These configurations overlap.
Two new tests verify default severity/target filtering and an explicit filter
that enables mesh TRACE while replacing the defaults. The release viewer
build with `debugger_capture` passes.

`analyze.mjs` checks all 192 requested/observed seeks, final clocks, dimensions,
readiness, camera/grid/environment/residency metadata, zero missing mesh
assets, event levels, syscall counts and all RGBA bytes including alpha.
All six capture processes exit zero. The strict image results are:

| Comparison | Differing pixels | Maximum RGB difference | Maximum alpha difference |
| --- | ---: | ---: | ---: |
| Saved bird TRACE versus INFO | 0 | 0 | 0 |
| Saved cow TRACE versus INFO | 0 | 0 | 0 |
| Saved bird INFO versus candidate default | 0 | 0 | 0 |
| Saved bird TRACE versus candidate explicit TRACE | 207 | 2 | 0 |
| Candidate default versus explicit TRACE | 207 | 2 | 0 |

Both candidate bird images were inspected and show the fully framed bird.
Matching metadata does not explain the 207-pixel differences. They remain
strict failures; neither a logging-induced cause nor a renderer fix is
established. No tolerance is relaxed and no rerun replaces these results.
The analyzer retains detailed differences and returns failure.

Every capture also retains four matched Vulkan/clipboard warnings, so both
capture drivers return failure despite successful processes. The Vulkan
loader skips the DZN ICD after error -9; the private compositor lacks the
clipboard protocol and the X11 fallback is unreachable. Other raw host
output is retained rather than classified as clean.

`analysis.json`, raw syscall summaries, application/viewer logs, environment
records, native images and capture metadata contain the results. Entry-asset
and executable manifests verify afterward; the entry manifest is not a full
cow dependency-closure check. Source and dependency identity:

- Parent: `9e7b65e0207eecb5d97dbc09f126c55c814b1278`.
- Saved viewer SHA-256: `e02bc26c3b07457e59a39f3f464e74e134ff249a37904f19ccbfff93b9dfd254`.
- Candidate viewer SHA-256: `b6cd2dd315821689ed2afd2ec9b2bd638417cd7b159a7ebeb742c29560244020`.
- Candidate `src/main.rs`: `6323596500731969a7003f75c62ff23b110d38f04196d69106b60a4049200741`.
- Unchanged lockfile: `818d95e2b264ce2bbda3fe8be369204f63b46157e9b0a861ff1c3d3cf35019db`.

Build/test commands use `nix develop --command oslo make fmt`, `harden` and
`build-debugger-capture`, with two Cargo jobs and dev/test debug info disabled.
Saved capture/analyzer scripts run through `oslo make run-headless-benchmark`
without rebuilding. `candidate.patch` records the tested source delta.

The current run ends at this local checkpoint. Native 60 Hz, earlier visual
failures, loading regressions, resource/lifecycle, downstream and platform
acceptance remain open. No new cache, reduced-quality policy or global CPU
fallback is introduced, and nothing is pushed.
