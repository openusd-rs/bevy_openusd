#!/usr/bin/env bash
set -euo pipefail
[[ $# == 2 ]] || { echo 'usage: check_subdivision_connectivity.sh COMPARE_OSD NEW_OUTPUT_DIRECTORY' >&2; exit 2; }
tool=$(realpath "$1")
[[ -x "$tool" ]] || { echo 'compare_osd must be executable' >&2; exit 2; }
mkdir "$2"
output=$(realpath "$2")
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
for name in Disconnected Connected; do
    for level in 0 1; do
        arguments=(assets/subdivision_connectivity.usda "/$name" "$output/$name-$level")
        if [[ "$level" == 1 ]]; then arguments+=(1); fi
        printf -v args '%q ' "${arguments[@]}"
        args=${args//\$/\$\$}
        make run RUN_WITH= CARGO="${CARGO:-cargo --offline}" APP_TARGET='--example normal_isolation' \
            ARGS="$args" >"$output/$name-$level.log" 2>&1
    done
    "$tool" "$output/$name-0/with_normals.usda" "$output/$name-1/with_normals.usda" \
        --compare-normals >"$output/$name-native.log" 2>&1
    cat "$output/$name-native.log"
done
echo 'SUBDIVISION_CONNECTIVITY_CHECK_OK'
