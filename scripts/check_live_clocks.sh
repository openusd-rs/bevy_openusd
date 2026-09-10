#!/bin/bash
set -euo pipefail

if [[ $# != 1 ]]; then
    echo "usage: check_live_clocks.sh NEW_OUTPUT_DIRECTORY" >&2
    exit 2
fi
output=$(realpath -m "$1")
[[ ! -e "$output" ]] || { echo "output directory must be new" >&2; exit 2; }
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
mkdir -p "$(dirname "$output")"
mkdir "$output"
unset USD_CPU_SKINNING USD_CAPTURE_CAMERA USD_CAPTURE_DOME USD_SUBDIVISION_LEVELS USD_CURVE_STEPS USD_CAPTURE_INSTANCE_SPACING
export USD_CAPTURE_RENDERER=forward USD_CAPTURE_SHADOWS=off

run_example() {
    local example=$1 args
    shift
    printf -v args '%q ' "$@"
    args=${args//\$/\$\$}
    make run CARGO='cargo --offline' APP_TARGET="--example $example" ARGS="$args" "${run_options[@]}"
}

run_options=(RUN_WITH=)
run_example uv_transform_fixture "$output/fixture" > "$output/fixture.log" 2>&1
run_example scalar_texture_fixture "$output/scalar-fixture" > "$output/scalar-fixture.log" 2>&1
run_example emissive_texture_fixture "$output/emissive-fixture" > "$output/emissive-fixture.log" 2>&1
run_example color_texture_fixture "$output/rgb-fixture" > "$output/rgb-fixture.log" 2>&1
printf 'case\tstatus\n' > "$output/results.tsv"

run_case() {
    local name=$1 live=$2 reference=$3 cpu=$4
    run_options=()
    unset USD_CPU_SKINNING
    export USD_CAPTURE_INSTANCE_TIMES=0,10 USD_CAPTURE_SWAP_CLOCKS=1
    run_example viewer_capture "$live" "$output/$name-live.png" 0 0 1 8 0 1 0 > "$output/$name-live.log" 2>&1 || return 1
    export USD_CAPTURE_INSTANCE_TIMES=10,0 USD_CAPTURE_SWAP_CLOCKS=0
    if [[ "$cpu" == 1 ]]; then export USD_CPU_SKINNING=1; fi
    run_example viewer_capture "$reference" "$output/$name-reference.png" 0 0 1 8 0 1 0 > "$output/$name-reference.log" 2>&1 || return 1
    grep -qx 'clocks_reversed_after_ready_frames=30' "$output/$name-live.capture.txt" || return 1
    grep -qx 'clocks_reversed_after_ready_frames=0' "$output/$name-reference.capture.txt" || return 1
    for mode in live reference; do
        grep -Fxq 'instance_times=[10.0, 0.0]' "$output/$name-$mode.capture.txt" || return 1
        grep -qx 'hierarchy_visible_meshes=2' "$output/$name-$mode.capture.txt" || return 1
        if grep -Eq '(^|[[:space:]])(ERROR|WARN)([[:space:]]|$)' "$output/$name-$mode.log"; then return 1; fi
    done
    if [[ "$cpu" == 1 ]]; then
        grep -qx 'hierarchy_visible_gpu_morph_meshes=2' "$output/$name-live.capture.txt" || return 1
        grep -qx 'hierarchy_visible_flat_material_entities=2' "$output/$name-live.capture.txt" || return 1
        grep -qx 'hierarchy_visible_unique_flat_materials=1' "$output/$name-live.capture.txt" || return 1
        grep -qx 'hierarchy_visible_gpu_morph_meshes=0' "$output/$name-reference.capture.txt" || return 1
    fi
    run_options=(RUN_WITH=)
    run_example capture_compare "$output/$name-live.rgba" "$output/$name-reference.rgba" 0 1280 "$output/$name-diff.png" > "$output/$name-compare.log" 2>&1 || return 1
}

status=0
for name in uv texture colorspace morph scalar scalar_interface file_interface colorspace_interface emissive rgb_emissive rgb_diffuse rgb_alpha; do
    case "$name" in
        uv) live="$output/fixture/mapped.usda"; reference="$output/fixture/reference.usda"; cpu=0 ;;
        texture) live="$output/fixture/file_samples.usda"; reference="$output/fixture/file_reference.usda"; cpu=0 ;;
        colorspace) live="$output/fixture/color_space_samples.usda"; reference=$live; cpu=0 ;;
        morph) live="$root/assets/morph_animation.usda"; reference=$live; cpu=1 ;;
        scalar) live="$output/scalar-fixture/animated.usda"; reference="$output/scalar-fixture/animated_reference.usda"; cpu=0 ;;
        scalar_interface) live="$output/scalar-fixture/interface_animated.usda"; reference="$output/scalar-fixture/animated_reference.usda"; cpu=0 ;;
        file_interface) live="$output/scalar-fixture/file_interface_animated.usda"; reference="$output/scalar-fixture/animated_reference.usda"; cpu=0 ;;
        colorspace_interface) live="$output/scalar-fixture/colorspace_interface_animated.usda"; reference="$output/scalar-fixture/colorspace_reference.usda"; cpu=0 ;;
        emissive) live="$output/emissive-fixture/animated.usda"; reference="$output/emissive-fixture/animated_reference.usda"; cpu=0 ;;
        rgb_emissive|rgb_diffuse|rgb_alpha) semantic=${name#rgb_}; live="$output/rgb-fixture/${semantic}_interface_animated.usda"; reference="$output/rgb-fixture/${semantic}_animated_reference.usda"; cpu=0 ;;
    esac
    if run_case "$name" "$live" "$reference" "$cpu"; then result=ok; else result=failed; status=1; fi
    printf '%s\t%s\n' "$name" "$result" | tee -a "$output/results.tsv"
done
exit "$status"
