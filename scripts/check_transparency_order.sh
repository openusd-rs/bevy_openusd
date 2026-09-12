#!/bin/bash
set -euo pipefail
[[ $# == 1 ]] || { echo 'usage: check_transparency_order.sh NEW_OUTPUT_DIRECTORY' >&2; exit 2; }
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
output=$(realpath -m "$1")
mkdir "$output"
cd "$root"
run_example() {
    local example=$1 args
    shift
    printf -v args '%q ' "$@"
    args=${args//\$/\$\$}
    make run CARGO='cargo --offline' APP_TARGET="--example $example" ARGS="$args"
}
for mode in prepass oit; do
    for order in '' _reversed; do
        USD_CAPTURE_RENDERER=$mode USD_CAPTURE_SHADOWS=off run_example viewer_capture \
            "assets/transparency_order$order.usda" "$output/$mode$order.png" 0 0 1 5 0 1 0 \
            >"$output/$mode$order.log" 2>&1
        grep -q '^CAPTURE_OK ' "$output/$mode$order.log"
    done
    status=0
    run_example capture_compare "$output/$mode.rgba" "$output/${mode}_reversed.rgba" \
        0 1280 "$output/$mode-diff.png" >"$output/$mode-compare.log" 2>&1 || status=$?
    if [[ "$mode" == oit ]]; then
        [[ "$status" == 0 ]]
        grep -q 'pixels=921600 changed=0 ' "$output/$mode-compare.log"
    else
        [[ "$status" != 0 ]]
        grep -Eq 'pixels=921600 changed=[1-9][0-9]* ' "$output/$mode-compare.log"
    fi
done
echo 'TRANSPARENCY_ORDER_OK: OIT invariant; ordinary blending order-dependent'
