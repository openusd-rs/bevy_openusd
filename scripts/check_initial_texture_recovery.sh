#!/bin/bash
set -euo pipefail

[[ $# == 1 ]] || { echo "usage: check_initial_texture_recovery.sh NEW_OUTPUT_DIRECTORY" >&2; exit 2; }
for command in weston weston-screenshooter setsid make realpath timeout; do
    command -v "$command" >/dev/null || { echo "missing command: $command" >&2; exit 2; }
done
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
output=$(realpath -m "$1")
[[ ! -e "$output" && ! -L "$output" ]] || { echo "output directory must be new" >&2; exit 2; }
mkdir -p "$(dirname "$output")"
mkdir "$output" "$output/initial" "$output/recovered"
cd "$root"
run_example() {
    local example=$1 args
    shift
    printf -v args '%q ' "$@"
    args=${args//\$/\$\$}
    make run RUN_WITH= CARGO="${CARGO:-cargo --offline}" APP_TARGET="--example $example" ARGS="$args"
}
run_example color_texture_fixture "$output/fixture" > "$output/fixture.log" 2>&1
cp "$output/fixture/diffuse_mapped.usda" "$output/scene.usda"
runtime=$(mktemp -d /dev/shm/usd-initial-recovery.XXXXXX)
chmod 700 "$runtime"
export XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY=usd-recovery
unset WAYLAND_SOCKET USD_UI_REPLAY USD_CAPTURE_CAMERA USD_VIEWER_SELECT
compositor_pid= viewer_pid=
cleanup() {
    [[ -z "$viewer_pid" ]] || kill -TERM -- "-$viewer_pid" 2>/dev/null || true
    [[ -z "$compositor_pid" ]] || kill -TERM -- "-$compositor_pid" 2>/dev/null || true
    wait 2>/dev/null || true
    rm -rf -- "$runtime"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
compositor=(weston --backend=headless --renderer=vulkan --fake-seat --width=1600 --height=1000 --socket="$WAYLAND_DISPLAY" --no-config --debug --idle-time=0)
if command -v nixVulkan >/dev/null; then compositor=(nixVulkan "${compositor[@]}"); fi
setsid "${compositor[@]}" > "$output/weston.log" 2>&1 &
compositor_pid=$!
for ((i=0;i<100;i++)); do
    [[ ! -S "$runtime/$WAYLAND_DISPLAY" ]] || break
    kill -0 "$compositor_pid"
    sleep 0.1
done
[[ -S "$runtime/$WAYLAND_DISPLAY" ]] || { echo "compositor startup timed out" >&2; exit 1; }
printf -v args '%q' "$output/scene.usda"
args=${args//\$/\$\$}
setsid env USD_WATCH_TEXTURES=1 USD_UI_CAPTURE_HANDSHAKE=1 USD_VIEWER_PANE=outliner USD_CAPTURE_TIME=0 USD_CAPTURE_DELAY_MS=3000 USD_SCREENSHOT="$output/recovered.viewport.png" \
    make run CARGO="${CARGO:-cargo --offline}" APP_TARGET='--bin usdview --features file_watcher' WAYLAND_DISPLAY="$WAYLAND_DISPLAY" ARGS="$args" > "$output/viewer.log" 2>&1 &
viewer_pid=$!
for ((i=0;i<300;i++)); do
    grep -qx USD_VIEWER_UI_UPDATED "$output/viewer.log" && break
    kill -0 "$viewer_pid"
    sleep 1
done
grep -qx USD_VIEWER_UI_UPDATED "$output/viewer.log"
sleep 5
capture() {
    kill -0 "$viewer_pid"
    (cd "$output/$1"; timeout --kill-after=1 15 weston-screenshooter > capture.log 2>&1)
    local files=("$output/$1"/wayland-screenshot-*.png)
    [[ ${#files[@]} == 1 && -f ${files[0]} ]]
    cp "${files[0]}" "$output/$1.png"
}
[[ ! -e "$output/white.png" ]]
capture initial
printf 'viewer_process_group=%s\nrepair_time=%s\n' "$viewer_pid" "$(date -Is)" > "$output/recovery.txt"
cp "$output/fixture/white.png" "$output/white.png"
for ((i=0;i<45;i++)); do
    grep -Fxq "VIEWPORT_CAPTURE_OK $output/recovered.viewport.png" "$output/viewer.log" && break
    kill -0 "$viewer_pid"
    sleep 1
done
grep -Fxq "VIEWPORT_CAPTURE_OK $output/recovered.viewport.png" "$output/viewer.log"
sleep 3
capture recovered
printf 'same_viewer_process_group_alive=%s\n' "$viewer_pid" >> "$output/recovery.txt"
width= height=
while IFS='=' read -r key value; do
    case "$key" in width) width=$value ;; height) height=$value ;; esac
done < "$output/recovered.viewport.capture.txt"
[[ "$width" =~ ^[1-9][0-9]*$ && "$height" =~ ^[1-9][0-9]*$ ]]
run_example capture_inspect "$output/recovered.viewport.png" 0 0 "$width" "$height" > "$output/viewport.inspect.log" 2>&1
echo "RECOVERY_UI_CAPTURE_OK $output (inspect initial, recovered and viewport images; logs may contain warnings)"
