#!/bin/bash
set -euo pipefail

[[ $# == 1 ]] || { echo "usage: check_retimed_instances.sh NEW_OUTPUT_DIRECTORY" >&2; exit 2; }
output=$(realpath -m "$1")
[[ ! -e "$output" ]] || { echo "output directory must be new" >&2; exit 2; }
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
mkdir -p "$output"
unset USD_CPU_SKINNING USD_CAPTURE_CAMERA USD_CAPTURE_DOME USD_SUBDIVISION_LEVELS USD_CURVE_STEPS
unset USD_CAPTURE_INSTANCE_TIMES USD_CAPTURE_SWAP_CLOCKS
export USD_CAPTURE_RENDERER=forward USD_CAPTURE_SHADOWS=off
printf 'time\tstatus\n' > "$output/results.tsv"

for time in 10 20 30; do
    for kind in instanced reference; do
        asset=assets/retimed_instances.usda
        [[ "$kind" != reference ]] || asset=assets/retimed_instances_reference.usda
        printf -v args '%q ' "$asset" "$output/$kind-$time.png" "$time" 10 8 18 0 0 0
        args=${args//\$/\$\$}
        make run CARGO='cargo --offline' APP_TARGET='--example viewer_capture' ARGS="$args" \
            > "$output/$kind-$time.log" 2>&1
        grep -qx 'hierarchy_visible_meshes=3' "$output/$kind-$time.capture.txt"
        grep -Fxq "instance_times=[$time.0]" "$output/$kind-$time.capture.txt"
        if grep -Eq '(^|[[:space:]])(ERROR|WARN)([[:space:]]|$)' "$output/$kind-$time.log"; then
            echo "unexpected renderer diagnostic: $kind at $time" >&2; exit 1
        fi
    done
    printf -v args '%q ' "$output/instanced-$time.rgba" "$output/reference-$time.rgba" 0 1280 "$output/diff-$time.png"
    args=${args//\$/\$\$}
    make run RUN_WITH= CARGO='cargo --offline' APP_TARGET='--example capture_compare' ARGS="$args" \
        > "$output/compare-$time.log" 2>&1
    printf '%s\tok\n' "$time" | tee -a "$output/results.tsv"
done
