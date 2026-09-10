#!/usr/bin/env bash
set -euo pipefail
tool=$(realpath "${1:?compiled compare-osd executable required}")
collection=$(realpath "${2:?USD collection directory required}")
output=${3:?new output directory required}
mkdir "$output"
output=$(realpath "$output")
cargo=${CARGO:-cargo --offline}
for name in ur5 spot; do
    case "$name" in
        ur5)
            asset="$collection/matlab/ur_description/ur5.usdc"
            prim=/ur5/world/base_link/shoulder_link/visual_0/geom
            ;;
        spot)
            asset="$collection/oems/spot_boston_dynamics/spot_base_urdf/spot.usdc"
            prim=/spot/MeshLibrary/base_obj
            ;;
    esac
    sha256sum "$asset" >"$output/$name-source.sha256"
    for level in 0 1; do
        refinement=
        if [[ "$level" == 1 ]]; then refinement=1; fi
        make run RUN_WITH= CARGO="$cargo" APP_TARGET='--example normal_isolation' \
            ARGS="'$asset' '$prim' '$output/$name-$level' $refinement" \
            >"$output/$name-$level.log" 2>&1
    done
    "$tool" "$output/$name-0/with_normals.usda" "$output/$name-1/with_normals.usda" \
        --compare-normals >"$output/$name-native.log" 2>&1
    cat "$output/$name-native.log"
done
echo 'SUBDIVISION_ASSET_CHECK_OK'
