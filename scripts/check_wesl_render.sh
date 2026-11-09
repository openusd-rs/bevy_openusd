#!/usr/bin/env bash
set -euo pipefail
[[ $# == 1 ]] || { echo 'usage: check_wesl_render.sh NEW_OUTPUT_DIRECTORY' >&2; exit 2; }
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
output=$(realpath -m "$1")
mkdir "$output"
cd "$root"
capture() {
    local name=$1 scene=$2 mode=$3 args
    shift 3
    printf -v args '%q ' "$scene" "$output/$name.png" 0 "$@"
    USD_CAPTURE_RENDERER=$mode USD_WGPU_VALIDATION=1 USD_WGPU_DEBUG=1 \
        oslo make run-example --name viewer_capture --args "$args" >"$output/$name.log" 2>&1
    grep -q '^CAPTURE_OK ' "$output/$name.log"
    bash scripts/check_viewer_log.sh "$output/$name.log"
}
for mode in forward prepass deferred oit; do
    capture "materials-$mode" assets/wesl_materials.usda "$mode" 0 3 8 0 0.7 0
done
USD_CAPTURE_GPU_INSTANCING=0 capture instancing assets/wesl_materials.usda forward 0 3 8 0 0.7 0
for mode in forward prepass deferred; do
    capture "morph-$mode" assets/morph_subsets.usda "$mode" 0 2 7 0 1 0
    capture "skel-$mode" assets/skel_morph_subsets.usda "$mode" 0 2 7 0 1 0
done
USD_CAPTURE_DOME=1 capture dome assets/dome_directional.usda forward
bash scripts/check_transparency_order.sh "$output/transparency"
echo "WESL_RENDER_OK $output (pipeline/readback checks; inspect images separately)"
