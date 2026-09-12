#!/bin/bash
set -euo pipefail
[[ $# -ge 2 && $# -le 4 ]] || { echo 'usage: capture_native_storm.sh ASSET NEW_OUTPUT_DIRECTORY [CAMERA] [TIME]' >&2; exit 2; }
asset=$(realpath "$1")
[[ ! -e "$2" && ! -L "$2" ]] || { echo 'output directory must be new' >&2; exit 2; }
output=$(realpath -m "$2")
[[ -f "$asset" && ! -e "$output" && ! -L "$output" ]] || { echo 'asset must exist; output directory must be new' >&2; exit 2; }
seconds=${USD_NATIVE_CAPTURE_TIMEOUT:-120}
[[ "$seconds" =~ ^[1-9][0-9]{0,2}$ && "$seconds" -le 300 ]] || { echo 'USD_NATIVE_CAPTURE_TIMEOUT must be 1..300 seconds' >&2; exit 2; }
time=${4:-0}
[[ "$time" =~ ^-?[0-9]+([.][0-9]+)?$ ]] || { echo 'TIME must be a finite decimal time code' >&2; exit 2; }
for command in weston usdrecord timeout setsid realpath grep sed; do command -v "$command" >/dev/null || { echo "missing command: $command" >&2; exit 2; }; done
root=$(cd "$(dirname "$0")/.." && pwd)
source "$root/scripts/capture_session.sh"
mkdir -p "$(dirname "$output")"
mkdir -- "$output"
runtime=$(mktemp -d /dev/shm/usd-storm.XXXXXX)
chmod 700 "$runtime"
xpid=; rpid=
cleanup() {
    capture_session_stop "$runtime/record.pid" "$rpid"
    capture_session_stop "$runtime/compositor.pid" "$xpid"
    wait 2>/dev/null || true
    rm -rf -- "$runtime"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
export XDG_RUNTIME_DIR="$runtime"
unset WAYLAND_SOCKET WAYLAND_DISPLAY
gpu=()
if command -v nixVulkan >/dev/null; then gpu=(nixVulkan); fi
capture_session_run "$runtime/compositor.pid" "${gpu[@]}" weston --backend=headless --renderer=vulkan --fake-seat --width=1440 --height=920 --socket=usd-storm --no-config --idle-time=0 --xwayland > "$output/weston.log" 2>&1 &
xpid=$!
for ((i=0; i<100; ++i)); do
    grep -q 'xserver listening on display' "$output/weston.log" && break
    kill -0 "$xpid"
    sleep .1
done
display=$(sed -n 's/.*xserver listening on display \(:[0-9]*\).*/\1/p' "$output/weston.log" | tail -1)
[[ "$display" =~ ^:[0-9]+$ ]] || { echo 'Xwayland startup timed out' >&2; exit 1; }
printf 'display=%s\nasset=%s\ntime=%s\ntimeout_seconds=%s\n' "$display" "$asset" "$time" "$seconds" > "$output/settings.txt"
camera=()
if [[ $# -ge 3 ]]; then camera=(--camera "$3"); fi
capture_session_run "$runtime/record.pid" env DISPLAY="$display" QT_QPA_PLATFORM=xcb \
    timeout --kill-after=5 "$seconds" "${gpu[@]}" usdrecord --renderer Storm "${camera[@]}" --frames "$time" --imageWidth 1280 \
    "$asset" "$output/frame.###.#########.png" > "$output/record.log" 2>&1 &
rpid=$!
set +e
wait "$rpid"
status=$?
set -e
printf 'usdrecord_exit=%s\n' "$status" | tee "$output/status.txt"
rpid=
[[ "$status" == 0 ]] || exit "$status"
frames=("$output"/frame.*.png)
[[ ${#frames[@]} == 1 && -s "${frames[0]}" ]] || { echo 'native renderer must produce exactly one image' >&2; exit 1; }
mv -- "${frames[0]}" "$output/frame.png"
echo "NATIVE_STORM_CAPTURE_OK $output/frame.png (inspect image and renderer diagnostics)"
