#!/usr/bin/env bash
set -euo pipefail
tool=$(realpath "${1:?compiled compare-osd executable required}")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cat >"$work/cage.usda" <<'USD'
#usda 1.0
def Mesh "Probe" {
    point3f[] points = [(0,0,0),(1,0,0),(0,1,0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0,1,2]
}
USD
cat >"$work/refined.usda" <<'USD'
#usda 1.0
def Mesh "Probe" {
    point3f[] points = [(0,0,0),(1,0,0),(0,1,0),(0.333333333,0.333333333,0),(0.5,0,0),(0,0.5,0),(0.5,0.5,0)]
    int[] faceVertexCounts = [3,3,3,3,3,3]
    int[] faceVertexIndices = [0,4,3,0,3,5,1,6,3,1,3,4,2,5,3,2,3,6]
    normal3f[] normals = [(0,0,1),(0,0,1),(0,0,1),(0,0,1),(0,0,1),(0,0,1),(0,0,1)]
}
USD
"$tool" "$work/cage.usda" "$work/refined.usda" --normal-probe "$work/output" >"$work/result"
grep -q 'zero_referenced_normals=0' "$work/result"
cmp <(grep -v 'normal3f\[\] normals' "$work/refined.usda") \
    <(grep -v 'normal3f\[\] normals' "$work/output/with_limit_normals.usda")
cmp <(grep 'normal3f\[\] normals' "$work/refined.usda") \
    <(grep 'normal3f\[\] normals' "$work/output/with_limit_normals.usda" | sed 's/-0/0/g')
cmp <(grep -v -E 'normal3f\[\] normals|point3f\[\] points' "$work/refined.usda") \
    <(grep -v -E 'normal3f\[\] normals|point3f\[\] points' "$work/output/with_limit_surface.usda")
expect_invalid() {
    local status=0
    "$tool" "$@" >"$work/invalid.log" 2>&1 || status=$?
    test "$status" -eq 2
}
expect_invalid
expect_invalid "$work/missing" "$work/missing"
expect_invalid "$work/cage.usda" "$work/cage.usda"
expect_invalid "$work/cage.usda" "$work/refined.usda" -1
expect_invalid "$work/cage.usda" "$work/refined.usda" --normal-probe "$work/output"
echo 'SUBDIVISION_REFERENCE_CHECK_OK'
