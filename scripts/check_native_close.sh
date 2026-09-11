#!/bin/bash
set -euo pipefail
[[ $# == 1 ]] || { echo 'usage: check_native_close.sh NEW_OUTPUT_DIRECTORY' >&2; exit 2; }
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
output=$(realpath -m "$1")
[[ ! -e "$output" && ! -L "$output" ]] || { echo 'output must be new' >&2; exit 2; }
for command in weston cc pkg-config setsid; do command -v "$command" >/dev/null; done
mkdir -p "$output"
make --eval='build-close-probe:; @cc -Wall -Wextra -Werror scripts/send_window_close.c -o target/send_window_close $$(pkg-config --cflags --libs x11)' build-close-probe
xpid=; vpid=
runtime=$(mktemp -d /dev/shm/usd-native-close.XXXXXX)
chmod 700 "$runtime"
cleanup() {
    [[ -z "$vpid" ]] || kill -TERM -- "-$vpid" 2>/dev/null || true
    [[ -z "$xpid" ]] || kill -TERM -- "-$xpid" 2>/dev/null || true
    wait 2>/dev/null || true
    rm -rf -- "$runtime"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
export XDG_RUNTIME_DIR="$runtime"
unset WAYLAND_SOCKET WAYLAND_DISPLAY
compositor=(weston --backend=headless --renderer=vulkan --fake-seat --width=1440 --height=920 --socket=usd-close --no-config --idle-time=0 --xwayland)
if command -v nixVulkan >/dev/null; then compositor=(nixVulkan "${compositor[@]}"); fi
setsid "${compositor[@]}" >"$output/weston.log" 2>&1 & xpid=$!
for ((i=0; i<100; ++i)); do
    grep -q 'xserver listening on display' "$output/weston.log" && break
    kill -0 "$xpid"
    sleep .1
done
display=$(sed -n 's/.*xserver listening on display \(:[0-9]*\).*/\1/p' "$output/weston.log" | tail -1)
[[ "$display" =~ ^:[0-9]+$ ]]
printf '%s\n' "$display" > "$output/display"
setsid env -u WAYLAND_DISPLAY WGPU_BACKEND=vulkan USD_UI_CAPTURE_HANDSHAKE=1 \
    USD_HOST_SCREENSHOT="$output/dialog.png" USD_HOST_SCREENSHOT_DELAY_MS=20000 \
    make run BACKEND=x11 DISPLAY="$display" CARGO='cargo --offline' \
    ARGS='assets/flagship_showcase.usda' >"$output/viewer.log" 2>&1 & vpid=$!
for ((i=0; i<60; ++i)); do
    grep -qx 'USD_VIEWER_UI_UPDATED' "$output/viewer.log" && break
    kill -0 "$vpid"
    sleep 1
done
grep -qx 'USD_VIEWER_UI_UPDATED' "$output/viewer.log"
sleep 5
target/send_window_close "$display" Mara | tee "$output/close.log"
sleep 20
kill -0 "$vpid"
grep 'HOST_CAPTURE_OK' "$output/viewer.log"
echo 'OS_CLOSE_VIEWER_SURVIVED; inspect dialog.png to verify the confirmation'
