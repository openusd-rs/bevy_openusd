#!/bin/bash
set -euo pipefail

[[ $# == 1 || $# == 2 ]] || { echo "usage: check_clipped_sphere.sh NEW_OUTPUT_DIRECTORY [clipped_sphere|switching_clips]" >&2; exit 2; }
fixture=${2:-clipped_sphere}
[[ "$fixture" == clipped_sphere || "$fixture" == switching_clips ]] || { echo "unknown clip fixture" >&2; exit 2; }
output=$(realpath -m "$1")
[[ ! -e "$output" ]] || { echo "output directory must be new" >&2; exit 2; }
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
mkdir -p "$output"
unset USD_CPU_SKINNING USD_CAPTURE_CAMERA USD_CAPTURE_DOME USD_SUBDIVISION_LEVELS USD_CURVE_STEPS
unset USD_CAPTURE_INSTANCE_TIMES USD_CAPTURE_SWAP_CLOCKS USD_CAPTURE_INSTANCE_SPACING
export USD_CAPTURE_RENDERER=forward USD_CAPTURE_SHADOWS=off
printf 'case\tstatus\n' > "$output/results.tsv"

for case in 0 5 10 20 clocks; do
    time=$case
    meshes=1
    camera='8 6 14 0 0 0'
    times="[$time.0]"
    [[ "$fixture" != switching_clips ]] || camera='0 10 32 0 0 0'
    if [[ "$case" == clocks ]]; then
        time=0
        meshes=2
        camera='0 8 22 0 0 0'
        times='[20.0, 10.0]'
        export USD_CAPTURE_INSTANCE_SPACING=8
        if [[ "$fixture" == switching_clips ]]; then
            camera='0 15 55 0 0 0'
            export USD_CAPTURE_INSTANCE_SPACING=18
        fi
    fi
    for kind in clipped reference; do
        asset="assets/$fixture.usda"
        [[ "$kind" != reference ]] || asset="assets/${fixture}_reference.usda"
        if [[ "$case" == clocks ]]; then
            export USD_CAPTURE_INSTANCE_TIMES=10,20 USD_CAPTURE_SWAP_CLOCKS=1
            if [[ "$kind" == reference ]]; then
                export USD_CAPTURE_INSTANCE_TIMES=20,10 USD_CAPTURE_SWAP_CLOCKS=0
            fi
        fi
        printf -v args '%q ' "$asset" "$output/$case-$kind.png" "$time"
        args="${args//\$/\$\$}$camera"
        make run CARGO='cargo --offline' APP_TARGET='--example viewer_capture' ARGS="$args" \
            > "$output/$case-$kind.log" 2>&1
        grep -qx "hierarchy_visible_meshes=$meshes" "$output/$case-$kind.capture.txt"
        grep -Fxq "instance_times=$times" "$output/$case-$kind.capture.txt"
        if grep -Eq '(^|[[:space:]])(ERROR|WARN)([[:space:]]|$)' "$output/$case-$kind.log"; then
            echo "unexpected renderer diagnostic: $case-$kind" >&2; exit 1
        fi
    done
    printf -v args '%q ' "$output/$case-clipped.rgba" "$output/$case-reference.rgba" 0 1280 "$output/$case-diff.png"
    args=${args//\$/\$\$}
    make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--example capture_compare' ARGS="$args" \
        > "$output/$case-compare.log" 2>&1
    printf '%s\tok\n' "$case" | tee -a "$output/results.tsv"
done
grep -qx 'clocks_reversed_after_ready_frames=30' "$output/clocks-clipped.capture.txt"
grep -qx 'clocks_reversed_after_ready_frames=0' "$output/clocks-reference.capture.txt"
for kind in clipped reference; do
    grep -qx "instance_spacing=$USD_CAPTURE_INSTANCE_SPACING" "$output/clocks-$kind.capture.txt"
done
