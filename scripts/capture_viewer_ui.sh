#!/bin/bash
set -euo pipefail

if [[ $# != 2 ]]; then
    echo "usage: capture_viewer_ui.sh ASSET OUTPUT.png" >&2
    exit 2
fi
for command in weston weston-screenshooter setsid make realpath grep timeout; do
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
capture_timeout=${USD_UI_CAPTURE_TIMEOUT:-15}
[[ "$capture_timeout" =~ ^[1-9][0-9]*$ && "$capture_timeout" -le 120 ]] || {
    echo "USD_UI_CAPTURE_TIMEOUT must be 1..120 seconds" >&2; exit 2;
}
second_wait=${USD_UI_CAPTURE_SECOND_WAIT:-0}
[[ "$second_wait" =~ ^(0|[1-9][0-9]{0,2})$ && "$second_wait" -le 300 ]] || {
    echo "USD_UI_CAPTURE_SECOND_WAIT must be 0..300 seconds" >&2; exit 2;
}
second="${output%.png}.second.png"
if [[ "$second_wait" != 0 ]]; then
    for path in "$second" "${second%.png}.capture.log" "${second%.png}.inspect.log" "${second%.png}.settings.txt"; do
        [[ ! -e "$path" && ! -L "$path" ]] || { echo "second capture companion must be new: $path" >&2; exit 2; }
    done
fi
paired=${USD_UI_CAPTURE_VIEWPORT:-0}
[[ "$paired" == 0 || "$paired" == 1 ]] || {
    echo "USD_UI_CAPTURE_VIEWPORT must be 0 or 1" >&2; exit 2;
}
viewport="${output%.png}.viewport.png"
if [[ "$paired" == 1 ]]; then
    for path in "$viewport" "${viewport%.png}.rgba" "${viewport%.png}.capture.txt" "${viewport%.png}.inspect.log"; do
        [[ ! -e "$path" && ! -L "$path" ]] || { echo "viewport companion must be new: $path" >&2; exit 2; }
    done
fi
scene_graph=${USD_UI_CAPTURE_SCENE_GRAPH:-0}
[[ "$scene_graph" == 0 || "$scene_graph" == 1 ]] || {
    echo "USD_UI_CAPTURE_SCENE_GRAPH must be 0 or 1" >&2; exit 2;
}
if [[ "$scene_graph" == 1 ]]; then
    for command in weston-debug timeout; do
        command -v "$command" >/dev/null || { echo "missing command: $command" >&2; exit 2; }
    done
fi
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
printf 'scene_graph_requested=%s\n' "$scene_graph" >> "${output%.png}.settings.txt"
printf 'capture_timeout_seconds=%s\n' "$capture_timeout" >> "${output%.png}.settings.txt"
printf 'viewport_requested=%s\n' "$paired" >> "${output%.png}.settings.txt"
printf 'second_capture_wait_seconds=%s\n' "$second_wait" >> "${output%.png}.settings.txt"
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
if [[ "$paired" == 1 ]]; then
    viewer=(env "USD_SCREENSHOT=$viewport" "${viewer[@]}")
fi
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
viewport_ok=1
if [[ "$paired" == 1 ]]; then
    viewport_ok=0
    for ((i=0; i<capture_timeout; i++)); do
        grep -Fxq "VIEWPORT_CAPTURE_OK $viewport" "${output%.png}.viewer.log" && break
        grep -Fq "VIEWPORT_CAPTURE_ERROR $viewport:" "${output%.png}.viewer.log" && break
        kill -0 "$viewer_pid" 2>/dev/null || break
        sleep 1
    done
    if grep -Fxq "VIEWPORT_CAPTURE_OK $viewport" "${output%.png}.viewer.log"; then
        width= height=
        while IFS='=' read -r key value; do
            case "$key" in width) width=$value ;; height) height=$value ;; esac
        done < "${viewport%.png}.capture.txt"
        if [[ "$width" =~ ^[1-9][0-9]*$ && "$height" =~ ^[1-9][0-9]*$ ]]; then
            if make run RUN_WITH= CARGO="${CARGO:-cargo --offline}" APP_TARGET='--example capture_inspect' \
                ARGS="$(printf '%q' "$viewport") 0 0 $width $height" > "${viewport%.png}.inspect.log" 2>&1; then
                viewport_ok=1
            fi
        fi
    fi
    printf 'viewport_inspection_passed=%s\n' "$viewport_ok" >> "${output%.png}.settings.txt"
    [[ "$viewport_ok" == 1 ]] || echo "warning: viewport readback missing or failed inspection; continuing compositor capture" >&2
fi
if [[ "$scene_graph" == 1 ]]; then
    if timeout --kill-after=1 5 weston-debug scene-graph > "${output%.png}.scene-graph.log" 2>&1; then
        echo 'scene_graph_status=ok' >> "${output%.png}.settings.txt"
    else
        status=$?
        printf 'scene_graph_status=failed:%s\n' "$status" >> "${output%.png}.settings.txt"
        echo "warning: scene graph diagnostic failed ($status); continuing capture" >&2
    fi
fi
capture_frame() {
    local destination=$1 status
    local -a captures
    cd "$runtime" || return 1
    printf 'capture_start_script_seconds=%s\n' "$SECONDS" >> "${destination%.png}.settings.txt"
    if timeout --kill-after=1 "$capture_timeout" weston-screenshooter > "${destination%.png}.capture.log" 2>&1; then
        echo 'screenshot_status=ok' >> "${destination%.png}.settings.txt"
    else
        status=$?
        printf 'screenshot_status=failed:%s\n' "$status" >> "${destination%.png}.settings.txt"
        echo "screenshot command failed ($status): $destination" >&2
        return 1
    fi
    shopt -s nullglob
    captures=(wayland-screenshot-*.png)
    [[ ${#captures[@]} == 1 ]] || { echo "expected one captured output" >&2; return 1; }
    cp -- "${captures[0]}" "$destination" || return 1
    rm -- "${captures[0]}" || return 1
    printf 'capture_end_script_seconds=%s\n' "$SECONDS" >> "${destination%.png}.settings.txt"
}
capture_ok=1
if [[ "$second_wait" != 0 ]]; then cp -- "${output%.png}.settings.txt" "${second%.png}.settings.txt"; fi
capture_frame "$output" || capture_ok=0
if [[ "$second_wait" != 0 ]]; then
    for ((i=0; i<second_wait; i++)); do
        kill -0 "$viewer_pid" 2>/dev/null || { echo "viewer exited before second capture" >&2; exit 1; }
        sleep 1
    done
    capture_frame "$second" || capture_ok=0
fi
cd "$root"
outputs=("$output")
if [[ "$second_wait" != 0 ]]; then outputs+=("$second"); fi
for frame in "${outputs[@]}"; do
    if [[ ! -f "$frame" ]] || ! make run RUN_WITH= CARGO="${CARGO:-cargo --offline}" APP_TARGET='--example capture_inspect' \
        ARGS="$(printf '%q' "$frame") 110 110 1400 800" > "${frame%.png}.inspect.log" 2>&1; then
        echo "viewer capture failed region inspection; image and logs retained: $frame" >&2
        echo 'region_inspection_passed=0' >> "${frame%.png}.settings.txt"
        capture_ok=0
    else
        echo 'region_inspection_passed=1' >> "${frame%.png}.settings.txt"
        echo "UI_FRAME_INSPECTION_OK $frame"
    fi
done
[[ "$capture_ok" == 1 ]] || exit 1
[[ "$viewport_ok" == 1 ]] || { echo "paired viewport capture failed; artifacts retained" >&2; exit 1; }
echo "UI_CAPTURE_OK $output (inspect image; fixed wait is not render readiness)"
