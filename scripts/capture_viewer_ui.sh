#!/bin/bash
set -euo pipefail

if [[ $# != 2 ]]; then
    echo "usage: capture_viewer_ui.sh ASSET OUTPUT.png" >&2
    exit 2
fi
for command in weston weston-screenshooter setsid make realpath grep; do
    command -v "$command" >/dev/null || { echo "missing command: $command" >&2; exit 2; }
done
asset=$(realpath "$1")
output=$(realpath -m "$2")
[[ -f "$asset" && "$output" == *.png && ! -e "$output" ]] || {
    echo "asset must exist; output must be a new .png path" >&2; exit 2;
}
delay=${USD_UI_CAPTURE_WAIT:-20}
[[ "$delay" =~ ^[1-9][0-9]*$ && "$delay" -le 300 ]] || {
    echo "USD_UI_CAPTURE_WAIT must be 1..300 seconds" >&2; exit 2;
}
renderer=${USD_UI_COMPOSITOR_RENDERER:-vulkan}
case "$renderer" in
    vulkan) ;;
    pixman) echo "warning: pixman failed wgpu surface compatibility in local validation; this is not a validated fallback" >&2 ;;
    gl) echo "warning: GL compositor screenshots were vertically inverted in local validation; inspect orientation" >&2 ;;
    *) echo "USD_UI_COMPOSITOR_RENDERER must be vulkan, gl or pixman" >&2; exit 2 ;;
esac
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
mkdir -p "$(dirname "$output")"
printf 'compositor_renderer=%s\ncapture_wait_seconds=%s\noutput_width=1600\noutput_height=1000\ninspection_region=110,110,1400,800\nvalidation=near-black-only\n' \
    "$renderer" "$delay" > "${output%.png}.settings.txt"
runtime=$(mktemp -d /dev/shm/usd-viewer-ui.XXXXXX)
chmod 700 "$runtime"
compositor_pid=
viewer_pid=
cleanup() {
    [[ -z "$viewer_pid" ]] || kill -TERM -- "-$viewer_pid" 2>/dev/null || true
    [[ -z "$compositor_pid" ]] || kill -TERM -- "-$compositor_pid" 2>/dev/null || true
    wait 2>/dev/null || true
    rm -rf -- "$runtime"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
export XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY=usd-capture
if [[ ${USD_UI_CAPTURE_PRIVATE_BUS:-0} == 1 ]]; then
    export XDG_CONFIG_HOME="$runtime/config" XDG_DATA_HOME="$runtime/data" XDG_CACHE_HOME="$runtime/cache"
    mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
fi
unset WAYLAND_SOCKET
compositor=(weston --backend=headless --renderer="$renderer" --fake-seat --width=1600 --height=1000 --socket="$WAYLAND_DISPLAY" --no-config --debug --idle-time=0)
if command -v nixVulkan >/dev/null; then compositor=(nixVulkan "${compositor[@]}"); fi
setsid "${compositor[@]}" > "${output%.png}.weston.log" 2>&1 &
compositor_pid=$!
for ((i=0; i<100; i++)); do
    [[ ! -S "$runtime/$WAYLAND_DISPLAY" ]] || break
    kill -0 "$compositor_pid" 2>/dev/null || { echo "compositor failed" >&2; exit 1; }
    sleep 0.1
done
[[ -S "$runtime/$WAYLAND_DISPLAY" ]] || { echo "compositor startup timed out" >&2; exit 1; }
cd "$root"
viewer=(env USD_UI_CAPTURE_HANDSHAKE=1 make run WAYLAND_DISPLAY="$WAYLAND_DISPLAY" CARGO="${CARGO:-cargo --offline}" ARGS="$(printf '%q' "$asset")")
if [[ ${USD_UI_CAPTURE_PRIVATE_BUS:-0} == 1 ]]; then
    command -v dbus-run-session >/dev/null || { echo "missing command: dbus-run-session" >&2; exit 2; }
    viewer=(dbus-run-session -- "${viewer[@]}")
fi
setsid "${viewer[@]}" > "${output%.png}.viewer.log" 2>&1 &
viewer_pid=$!
for ((i=0; i<300; i++)); do
    grep -qx 'USD_VIEWER_UI_UPDATED' "${output%.png}.viewer.log" && break
    kill -0 "$viewer_pid" 2>/dev/null || { echo "viewer exited during startup" >&2; exit 1; }
    sleep 1
done
grep -qx 'USD_VIEWER_UI_UPDATED' "${output%.png}.viewer.log" || { echo "viewer first UI update timed out" >&2; exit 1; }
for ((i=0; i<delay; i++)); do
    kill -0 "$viewer_pid" 2>/dev/null || { echo "viewer exited before capture" >&2; exit 1; }
    sleep 1
done
cd "$runtime"
weston-screenshooter > "${output%.png}.capture.log" 2>&1
shopt -s nullglob
captures=(wayland-screenshot-*.png)
[[ ${#captures[@]} == 1 ]] || { echo "expected one captured output" >&2; exit 1; }
cp -- "${captures[0]}" "$output"
cd "$root"
make run RUN_WITH= CARGO="${CARGO:-cargo --offline}" APP_TARGET='--example capture_inspect' \
    ARGS="$(printf '%q' "$output") 110 110 1400 800" > "${output%.png}.inspect.log" 2>&1 || {
    echo "viewer capture failed region inspection; image and logs retained: $output" >&2; exit 1;
}
echo "UI_CAPTURE_OK $output (inspect image; fixed wait is not render readiness)"
