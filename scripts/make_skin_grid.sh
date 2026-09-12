#!/bin/bash
set -euo pipefail
[[ $# -ge 2 && $# -le 3 ]] || { echo 'usage: make_skin_grid.sh NEW_DIRECTORY CELLS_PER_SIDE [static|animated]' >&2; exit 2; }
case "${3:-static}" in
    static) base=skel_morph_blended_normals ;;
    animated) base=skel_morph_blended_animated ;;
    *) echo 'joint mode must be static or animated' >&2; exit 2 ;;
esac
[[ "$2" =~ ^[1-9][0-9]{0,2}$ && "$2" -le 256 ]] || { echo 'cells must be 1..256' >&2; exit 2; }
[[ ! -e "$1" && ! -L "$1" ]] || { echo 'output directory must be new' >&2; exit 2; }
root=$(cd "$(dirname "$0")/.." && pwd)
output=$(realpath -m "$1")
mkdir -p "$(dirname "$output")"
mkdir -- "$output"
for name in skel_morph_blended_animated skel_morph_blended_normals skel_morph_native_normals skel_morph_reference skel_morph_tangent_normals morph_tangent_normals skel_morph_normals morph_normals morph_subsets morph_animation blendshape_test; do
    cp -- "$root/assets/$name.usda" "$output/$name.usda"
done
awk -v n="$2" -v base="$base" 'BEGIN {
    print "#usda 1.0\n( upAxis = \"Y\"\n subLayers = [@" base ".usda@] )"
    print "over \"Test\"\n{\n over \"Face\"\n {"
    printf "point3f[] points = ["
    for(y=0;y<=n;y++) for(x=0;x<=n;x++) printf "%s(%.9g,%.9g,0)", (x+y ? "," : ""), x/n, y/n
    print "]"
    printf "normal3f[] normals = ["
    for(i=0;i<(n+1)*(n+1);i++) printf "%s(0,0,1)", (i ? "," : "")
    print "] (interpolation = \"vertex\")"
    printf "texCoord2f[] primvars:st = ["
    for(y=0;y<=n;y++) for(x=0;x<=n;x++) printf "%s(%.9g,%.9g)", (x+y ? "," : ""), x/n, y/n
    print "] (interpolation = \"vertex\")"
    printf "int[] faceVertexCounts = ["
    for(i=0;i<2*n*n;i++) printf "%s3", (i ? "," : "")
    print "]"
    printf "int[] faceVertexIndices = ["
    for(y=0;y<n;y++) for(x=0;x<n;x++) {
        a=y*(n+1)+x; b=a+1; c=a+n+2; d=a+n+1
        printf "%s%d,%d,%d,%d,%d,%d", (x+y ? "," : ""), a,b,c,a,c,d
    }
    print "]"
    printf "int[] primvars:skel:jointIndices = ["
    for(i=0;i<(n+1)*(n+1);i++) printf "%s0,1", (i ? "," : "")
    print "] (elementSize = 2\n interpolation = \"vertex\")"
    printf "float[] primvars:skel:jointWeights = ["
    for(y=0;y<=n;y++) for(x=0;x<=n;x++) printf "%s%.9g,%.9g", (x+y ? "," : ""), 1-x/n, x/n
    print "] (elementSize = 2\n interpolation = \"vertex\")\n }\n}"
}' > "$output/grid.usda"
printf 'SKIN_GRID_CREATED cells=%s points=%s triangles=%s path=%s/grid.usda\n' "$2" "$(( ($2+1)*($2+1) ))" "$(( 2*$2*$2 ))" "$output"
