#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
directory=$(mktemp -d)
trap 'rm -rf -- "$directory"' EXIT
mkdir "$directory/bin"
for command in weston weston-screenshooter; do
    printf '#!/bin/bash\nexit 90\n' > "$directory/bin/$command"
    chmod +x "$directory/bin/$command"
done
export PATH="$directory/bin:$PATH"
reject() {
    local status=0
    /bin/bash "$root/scripts/capture_viewer_ui.sh" "$root/assets/numeric_array.usda" "$1" > "$directory/result" 2>&1 || status=$?
    [[ "$status" == 2 ]] || { cat "$directory/result" >&2; echo "expected preflight exit 2, got $status" >&2; exit 1; }
    grep -q 'must be.*new' "$directory/result"
}
for suffix in settings.txt weston.log viewer.log capture.log inspect.log scene-graph.log; do
    for kind in file symlink; do
        folder="$directory/$suffix-$kind"
        mkdir "$folder"
        companion="$folder/capture.$suffix"
        if [[ "$kind" == file ]]; then printf 'retained evidence\n' > "$companion";
        else ln -s "$folder/missing" "$companion"; fi
        reject "$folder/capture.png"
        [[ ! -e "$folder/capture.png" && ! -e "$folder/missing" ]]
        if [[ "$kind" == file ]]; then [[ $(cat "$companion") == 'retained evidence' ]];
        else [[ -L "$companion" ]]; fi
        [[ $(find "$folder" -mindepth 1 -maxdepth 1 | wc -l) == 1 ]]
    done
done
mkdir "$directory/png"
ln -s "$directory/png/missing.png" "$directory/png/capture.png"
reject "$directory/png/capture.png"
[[ -L "$directory/png/capture.png" && ! -e "$directory/png/missing.png" ]]
[[ $(find "$directory/png" -mindepth 1 -maxdepth 1 | wc -l) == 1 ]]
echo CAPTURE_PREFLIGHT_TESTS_OK
