#!/bin/bash
set -euo pipefail

cube=${VKCUBE:-vkcube}
command -v "$cube" >/dev/null || { echo "missing Vulkan control: $cube" >&2; exit 2; }
command -v nixVulkan >/dev/null || { echo 'missing command: nixVulkan' >&2; exit 2; }
echo 'VKCUBE_CONTROL no Mara, egui, Bevy or USD; synthetic startup handshake'
echo 'USD_VIEWER_UI_UPDATED'
exec nixVulkan "$cube" --wsi wayland --width 1440 --height 920 --present_mode 1
