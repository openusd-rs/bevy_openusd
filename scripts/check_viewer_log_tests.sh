#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
directory=$(mktemp -d)
trap 'rm -rf -- "$directory"' EXIT
log="$directory/viewer log.txt"
check() {
    local expected=$1 actual=0
    shift
    bash "$root/scripts/check_viewer_log.sh" "$@" > "$directory/result" 2>&1 || actual=$?
    [[ "$actual" == "$expected" ]] || { cat "$directory/result" >&2; echo "expected $expected, got $actual" >&2; exit 1; }
}
printf 'USD_VIEWER_UI_UPDATED\nINFO normal operation\n' > "$log"
check 0 "$log"
for message in "thread 'main' panicked at source.rs:3:4:" "thread '<unnamed>' (123) panicked at source.rs:3:4:" \
    'Encountered a panic in system `example`!' 'thread caused non-unwinding panic. aborting.' 'fatal runtime error: stack overflow' \
    '2026-09-12T13:54:43Z ERROR bevy_render::error_handler: Caught rendering error: Out of Memory' \
    '2026-09-12T13:54:43Z ERROR bevy_render::error_handler: Caught rendering error: Validation Error' \
    '2026-11-09T22:15:39Z ERROR bevy_asset::server: Path not found: usd_bevy/route/strand_material.wesl' \
    '2026-11-09T22:15:42Z WARN bevy_shader::shader_cache: Shader `embedded://usd_bevy/route/strand_prepass.wesl` has an unresolved import:' \
    '2026-11-09T22:15:42Z ERROR bevy_render::render_resource::pipeline_cache: failed to compile shader'; do
    printf '%s\nUSD_VIEWER_UI_UPDATED\n' "$message" > "$log"
    check 1 "$log"
    ! grep -q 'VIEWER_LOG_OK' "$directory/result"
    printf 'CAPTURE_PIPELINES_READY compiled=12\nCAPTURE_OK frame.png\n' >> "$log"
    if [[ "$message" == *'WARN bevy_shader::shader_cache:'* ]]; then check 0 "$log";
    else check 1 "$log"; fi
done
warning='WARN bevy_shader::shader_cache: Shader `embedded://bevy_pbr/render/pbr.wesl` has an unresolved import:'
for proof in 'CAPTURE_PIPELINES_READY compiled=0' 'CAPTURE_PIPELINES_READY compiled=-1' \
    'CAPTURE_PIPELINES_READY compiled=12 pending=1' 'CAPTURE_PIPELINES_READY compiled=12 errors=1'; do
    printf '%s\n%s\nCAPTURE_OK frame.png\n' "$warning" "$proof" > "$log"
    check 1 "$log"
done
printf 'CAPTURE_PIPELINES_READY compiled=12\n%s\nCAPTURE_OK frame.png\n' "$warning" > "$log"
check 1 "$log"
printf '%s\nCAPTURE_OK frame.png\nCAPTURE_PIPELINES_READY compiled=12\n' "$warning" > "$log"
check 1 "$log"
printf '%s\nCAPTURE_PIPELINES_READY compiled=12\n' "$warning" > "$log"
check 1 "$log"
printf '%s\nCAPTURE_PIPELINES_READY compiled=12\nCAPTURE_OK frame.png\n%s\n' "$warning" "$warning" > "$log"
check 1 "$log"
printf 'ERROR bevy_shader::shader_cache: Shader `broken.wesl` has an unresolved import:\nCAPTURE_PIPELINES_READY compiled=12\nCAPTURE_OK frame.png\n' > "$log"
check 1 "$log"
printf 'INFO no panic detected\nINFO discussing panicked at as text\nWARN egui_winit::clipboard: Failed to initialize arboard clipboard\n' > "$log"
check 0 "$log"
: > "$log"
check 0 "$log"
check 2
check 2 "$directory/missing"
check 2 "$directory"
check 2 "$log" extra
echo VIEWER_LOG_TESTS_OK
