#!/usr/bin/env bash
set -euo pipefail
[[ $# == 3 || $# == 4 ]] || { echo 'usage: check_subdivision_meshes.sh COMPARE_OSD ASSET NEW_OUTPUT_DIRECTORY [PRIM_PREFIX]' >&2; exit 2; }
tool=$(realpath "$1")
asset=$(realpath "$2")
[[ -x "$tool" && -f "$asset" ]] || { echo 'executable and asset required' >&2; exit 2; }
mkdir "$3"
output=$(realpath "$3")
prefix=${4:-/}
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
sha256sum "$asset" >"$output/source.sha256"
printf -v args '%q ' "$asset" 0
args=${args//\$/\$\$}
make run RUN_WITH= CARGO="${CARGO:-cargo --offline}" APP_TARGET='--example scene_report' \
    ARGS="$args" >"$output/source.log" 2>&1
mapfile -t paths < <(sed -n 's/^\(\/.*\) subdivision_authored=.*/\1/p' "$output/source.log")
count=0
failures=0
for path in "${paths[@]}"; do
    [[ "$path" == "$prefix"* ]] || continue
    count=$((count + 1))
    probe="$output/mesh-$count"
    valid=1
    printf '%s\n' "$path" >"$probe.path"
    for level in 0 1; do
        arguments=("$asset" "$path" "$probe-$level")
        if [[ "$level" == 1 ]]; then arguments+=(1); fi
        printf -v args '%q ' "${arguments[@]}"
        args=${args//\$/\$\$}
        if ! make run RUN_WITH= CARGO="${CARGO:-cargo --offline}" APP_TARGET='--example normal_isolation' \
            ARGS="$args" >"$probe-$level.log" 2>&1; then valid=0; fi
    done
    if [[ "$valid" == 1 ]] && "$tool" "$probe-0/with_normals.usda" "$probe-1/with_normals.usda" \
        --compare-normals >"$probe-native.log" 2>&1; then
        printf 'PASS\t%s\n' "$path" | tee -a "$output/results.tsv"
    else
        failures=$((failures + 1))
        printf 'FAIL\t%s\n' "$path" | tee -a "$output/results.tsv"
    fi
done
printf 'meshes=%s failures=%s\n' "$count" "$failures" | tee "$output/summary.txt"
[[ "$count" -gt 0 && "$failures" == 0 ]]
