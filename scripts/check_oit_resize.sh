#!/bin/bash
set -euo pipefail
[[ $# == 1 ]] || { echo 'usage: check_oit_resize.sh NEW_OUTPUT_DIRECTORY' >&2; exit 2; }
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
source scripts/capture_session.sh
output=$(realpath -m "$1")
mkdir "$output"
make --eval='build-resize-probe:; @cc -Wall -Wextra -Werror scripts/resize_window.c -o target/resize_window $$(pkg-config --cflags --libs x11)' build-resize-probe
runtime=$(mktemp -d /dev/shm/usd-oit-resize.XXXXXX)
chmod 700 "$runtime"
xpid=; vpid=
cleanup() {
    capture_session_stop "$runtime/viewer.pid" "$vpid"
    capture_session_stop "$runtime/compositor.pid" "$xpid"
    rm -rf -- "$runtime"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
export XDG_RUNTIME_DIR="$runtime"
unset WAYLAND_SOCKET WAYLAND_DISPLAY
capture_session_run "$runtime/compositor.pid" nixVulkan weston --backend=headless --renderer=vulkan \
    --fake-seat --width=1440 --height=920 --socket=usd-resize --no-config --idle-time=0 --xwayland \
    >"$output/weston.log" 2>&1 & xpid=$!
for ((i=0; i<100; ++i)); do
    grep -q 'xserver listening on display' "$output/weston.log" && break
    kill -0 "$xpid"; sleep .1
done
display=$(sed -n 's/.*xserver listening on display \(:[0-9]*\).*/\1/p' "$output/weston.log" | tail -1)
[[ "$display" =~ ^:[0-9]+$ ]]
capture_session_run "$runtime/viewer.pid" env -u WAYLAND_DISPLAY -u USD_UI_REPLAY \
    WGPU_BACKEND=vulkan USD_VIEWER_OIT=1 USD_VIEWER_PANE=rendering USD_UI_CAPTURE_HANDSHAKE=1 \
    USD_HOST_SCREENSHOT="$output/restored.png" USD_HOST_SCREENSHOT_DELAY_MS=30000 \
    make run BACKEND=x11 DISPLAY="$display" CARGO='cargo --offline' \
    ARGS='assets/transparency_order.usda' >"$output/viewer.log" 2>&1 & vpid=$!
for ((i=0; i<60; ++i)); do
    grep -qx 'USD_VIEWER_UI_UPDATED' "$output/viewer.log" && break
    kill -0 "$vpid"; sleep 1
done
grep -qx 'USD_VIEWER_UI_UPDATED' "$output/viewer.log"
mapfile -t windows < <(xwininfo -display "$display" -root -tree | awk '/"Mara":/ {print $1}')
[[ ${#windows[@]} == 1 ]]
sleep 3
target/resize_window "$display" "${windows[0]}" 3840 2160 | tee "$output/resize.log"
sleep 5
xwininfo -display "$display" -id "${windows[0]}" > "$output/large-window.txt"
grep -q 'Width: 3840' "$output/large-window.txt"
grep -q 'Height: 2160' "$output/large-window.txt"
grep -Eq 'OIT disabled: [0-9]+x[0-9]+ target exceeds' "$output/viewer.log"
target/resize_window "$display" "${windows[0]}" 1440 920 | tee -a "$output/resize.log"
sleep 25
kill -0 "$vpid"
grep 'HOST_CAPTURE_OK' "$output/viewer.log"
echo 'OIT_RESIZE_GUARD_OK: oversized window survived; inspect restored.png'
