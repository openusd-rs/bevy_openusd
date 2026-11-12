#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 1 || ! -f "$1" || ! -r "$1" ]]; then
    echo "usage: check_viewer_log.sh READABLE_VIEWER_LOG" >&2
    exit 2
fi
status=0
grep -E "^thread .* panicked at|^Encountered a panic in system|^thread caused non-unwinding panic|^fatal runtime error:|ERROR .*bevy_render::error_handler: Caught rendering error:|ERROR .*bevy_render::render_resource::pipeline_cache:|ERROR .*bevy_shader::shader_cache: Shader .* has an unresolved import:|ERROR .*bevy_asset::server: Path not found: usd_bevy/route/.*\\.wesl" -- "$1" >/dev/null || status=$?
case "$status" in
    0) echo "viewer log contains a runtime panic or rendering failure; artifacts retained: $1" >&2; exit 1 ;;
    1) ;;
    *) echo "viewer log inspection failed: $1" >&2; exit 2 ;;
esac
status=0
awk '
    /bevy_shader::shader_cache: Shader .* has an unresolved import:/ { warning = NR }
    /^CAPTURE_PIPELINES_READY/ {
        if ($0 ~ /^CAPTURE_PIPELINES_READY compiled=[1-9][0-9]*$/) ready = NR
        else invalid = 1
    }
    /^CAPTURE_OK / { capture = NR }
    END { if (invalid || (warning && (ready <= warning || capture <= ready))) exit 1 }
' "$1" || status=$?
case "$status" in
    0) echo "VIEWER_LOG_OK $1 (recovered imports require compiled pipelines and readback)" ;;
    1) echo "viewer log lacks final pipeline/readback proof after a shader import warning: $1" >&2; exit 1 ;;
    *) echo "viewer log inspection failed: $1" >&2; exit 2 ;;
esac
