# Current release viewer: Fendt desktop check

At edb6875, rebuilt the release usdview and opened the user's current
`/home/bresilla/machines/usd/fendt_tractor.usdz` in the owned private Weston
capture session. The file is 81325209 bytes, SHA256
`6037fbe4bdb1d16da6d8a82dee8cb82815a27fbae986d5f825603e4853e57ceb`.
The hash is identical before and after capture. No original asset, existing
viewer process, saved desktop configuration or system compositor was changed.

Artifacts:

- `target/current-fendt-host.png`: inspected 1440x920 application capture.
- `target/current-fendt-desktop.png`: compositor capture at 45-second wait.
- `target/current-fendt-desktop.second.png`: inspected second desktop capture
  five seconds later; byte-identical to the first desktop image.
- `target/current-fendt-desktop.scene-graph.log`: private compositor's mapped
  view/output evidence.

The green tractor, red wheels, cabin, headlights, hierarchy/status panel and
grid are visible. Status reads Ready. Neither desktop frame is blank. This is
an idle current-viewer smoke check, not native material parity, interaction
coverage, animated frame pacing or proof that intermittent blackouts are fixed.

Both desktop inspection guards, the panic-log guard and host capture report
success. The log contains an arboard clipboard initialization warning about the
unavailable X11 server. The wrapper exited zero and cleaned up its own viewer
and compositor; the captured viewer is not left running on the user's desktop.
Logs: `/tmp/current-machine-viewer-build.log`, `/tmp/current-fendt-ui.log` and
`target/current-fendt-desktop.{viewer,weston,capture,inspect}.log`.

Reproduction (new output paths required):

```sh
make build CARGO='cargo --offline' APP_TARGET='--release --bin usdview'
USD_UI_CAPTURE_PROFILE=release USD_UI_CAPTURE_SCENE_GRAPH=1 USD_UI_CAPTURE_SECOND_WAIT=5 USD_HOST_SCREENSHOT="$PWD/target/NEW-host.png" USD_HOST_SCREENSHOT_DELAY_MS=35000 USD_UI_CAPTURE_WAIT=45 make --eval='machine-ui:; @/bin/bash scripts/capture_viewer_ui.sh /home/bresilla/machines/usd/fendt_tractor.usdz target/NEW-desktop.png' machine-ui
```

The installed Weston binary directory must be on PATH. No source code changed
and no additional full test suite was run for this visual check.
