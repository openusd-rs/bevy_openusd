#!/bin/bash
set -euo pipefail

if [[ $# -lt 4 ]]; then
    echo "usage: compare_deformation.sh ASSET NEW_OUTPUT_DIRECTORY RGB_TOLERANCE TIME [TIME...]" >&2
    exit 2
fi
asset=$(realpath "$1")
output=$(realpath -m "$2")
tolerance=$3
shift 3
[[ -f "$asset" && ! -e "$output" ]] || { echo "asset must exist and output directory must be new" >&2; exit 2; }
[[ "$tolerance" =~ ^[0-9]{1,3}$ ]] && ((10#$tolerance <= 255)) || { echo "RGB tolerance must be 0..255" >&2; exit 2; }
tolerance=$((10#$tolerance))
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
mkdir -p "$(dirname "$output")"
mkdir "$output"
printf 'case\ttime\tstatus\tdifference\n' > "$output/results.tsv"
status=0
index=0

run_example() {
    local example=$1 args
    shift
    printf -v args '%q ' "$@"
    args=${args//\$/\$\$}
    make run CARGO='cargo --offline' APP_TARGET="--example $example" ARGS="$args" "${run_options[@]}"
}

for time in "$@"; do
    printf -v prefix 'case-%03d' "$index"
    index=$((index + 1))
    result=ok
    for mode in gpu cpu; do
        run_options=()
        if [[ "$mode" == cpu ]]; then export USD_CPU_SKINNING=1; else unset USD_CPU_SKINNING; fi
        if ! run_example viewer_capture "$asset" "$output/$prefix-$mode.png" "$time" > "$output/$prefix-$mode.log" 2>&1; then
            result=capture_failed
        elif grep -Eq '(^|[[:space:]])(ERROR|WARN)([[:space:]]|$)' "$output/$prefix-$mode.log"; then
            result=renderer_diagnostic
        elif [[ ! -f "$output/$prefix-$mode.capture.txt" ]]; then
            result=missing_capture_metadata
        elif [[ "$mode" == gpu ]] && ! grep -Eq '^hierarchy_visible_gpu_(meshes|morph_meshes)=[1-9][0-9]*$' "$output/$prefix-$mode.capture.txt"; then
            result=no_gpu_deformation
        elif [[ "$mode" == cpu ]] && { ! grep -qx 'hierarchy_visible_gpu_meshes=0' "$output/$prefix-$mode.capture.txt" || ! grep -qx 'hierarchy_visible_gpu_morph_meshes=0' "$output/$prefix-$mode.capture.txt"; }; then
            result=invalid_cpu_deformation_control
        fi
    done
    difference=
    if [[ -f "$output/$prefix-gpu.rgba" && -f "$output/$prefix-cpu.rgba" ]]; then
        run_options=(RUN_WITH=)
        if ! run_example capture_compare "$output/$prefix-gpu.rgba" "$output/$prefix-cpu.rgba" "$tolerance" 1280 "$output/$prefix-diff.png" > "$output/$prefix-compare.log" 2>&1; then
            if [[ "$result" == ok ]]; then result=comparison_failed; fi
        fi
        difference=$(grep '^pixels=' "$output/$prefix-compare.log" || true)
        [[ -n "$difference" ]] || result=comparison_failed
    else
        result=capture_failed
    fi
    printf '%s\t%s\t%s\t%s\n' "$prefix" "$time" "$result" "$difference" | tee -a "$output/results.tsv"
    if [[ "$result" != ok ]]; then status=1; fi
done
exit "$status"
