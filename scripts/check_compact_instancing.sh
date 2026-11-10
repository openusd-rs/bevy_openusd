#!/usr/bin/env bash
set -euo pipefail
[[ $# == 1 ]] || { echo 'usage: check_compact_instancing.sh NEW_OUTPUT_DIRECTORY' >&2; exit 2; }
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
output=$(realpath -m "$1")
mkdir "$output"
cd "$root"
oslo make build-compact-tools > "$output/build.log" 2>&1
mkdir "$output/tools"
for name in compact_instancing capture_compare; do
    cp --reflink=auto "target/debug/examples/$name" "$output/tools/$name"
done
sha256sum "$output/tools/compact_instancing" "$output/tools/capture_compare" > "$output/binaries.sha256"
run_example() {
    local name=$1
    shift
    local command=("$output/tools/$name" "$@")
    if [[ ! -e /etc/NIXOS ]]; then command=(nixVulkan "${command[@]}"); fi
    env WGPU_VALIDATION="${USD_WGPU_VALIDATION:-${WGPU_VALIDATION:-0}}" \
        WGPU_DEBUG="${USD_WGPU_DEBUG:-${WGPU_DEBUG:-0}}" "${command[@]}"
}
for view in overview edge; do
    for mode in compact expanded; do
        log="$output/$view-$mode.log"
        USD_COMPACT_VIEW=$view USD_COMPACT_CULL_CHECK=1 USD_WGPU_VALIDATION=1 USD_WGPU_DEBUG=1 \
            run_example compact_instancing "$mode" 64 "$output/$view-$mode.png" 20 >"$log" 2>&1
        grep -q '^CAPTURE_OK ' "$log"
        grep -qx 'COMPACT_CULL_CHECK offscreen_visible=0' "$log"
        grep -Eq '^COMPACT_VISIBLE restored_visible=[1-9][0-9]*$' "$log"
        bash scripts/check_viewer_log.sh "$log"
    done
    run_example capture_compare "$output/$view-compact.png.rgba" \
        "$output/$view-expanded.png.rgba" 1 800 "$output/$view-diff.png" \
        >"$output/$view-compare.log" 2>&1
    grep -q '^pixels=480000 ' "$output/$view-compare.log"
done
echo "COMPACT_RENDER_OK $output (RGB tolerance 1; inspect images separately)"
