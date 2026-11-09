-- bevy_openusd's build, as recipes.
--
--   make            the recipes, with what each of them says it does
--   make build      the usdview binary
--   make run        usdview, with host graphics on NixOS or nixVulkan elsewhere
--   make test       the usdview tests
--
-- At an oslo prompt in this directory `make` is enough; everywhere else it is `oslo make`.
-- The toolchain and the non-NixOS `nixVulkan` wrapper come from `.env.lua`'s dev shell.

local make = oslo.make

local APP = "usdview"

-- Formats workspace crates only; `--all` would also reach vendored path deps.
local function fmt(...)
  sh.cargo("fmt", "-p", "usdview", "-p", "usd_bevy", "-p", "usd_macro", "-p", "usd_web", ...)
end

-- Picks a live Wayland socket per launch, else falls back to X11, then runs `cmd`.
local function fresh_display_env(cmd)
  return ([[
_usd_xdg_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
_usd_wl=""
case "${USD_WINDOW_BACKEND:-auto}" in
  auto|wayland|x11) ;;
  *) echo "USD_WINDOW_BACKEND must be auto, wayland or x11" >&2; exit 2 ;;
esac
if [ -n "${WAYLAND_DISPLAY:-}" ] && [ -S "$_usd_xdg_runtime/$WAYLAND_DISPLAY" ]; then
  _usd_wl="$WAYLAND_DISPLAY"
elif [ -S "$_usd_xdg_runtime/wayland-0" ]; then
  _usd_wl="wayland-0"
else
  _usd_wl=$(ls -t "$_usd_xdg_runtime"/wayland-* 2>/dev/null | grep -v '\.lock$' | head -n1 | xargs -r basename)
fi
if [ "${USD_WINDOW_BACKEND:-auto}" = x11 ]; then
  _usd_wl=""
elif [ "${USD_WINDOW_BACKEND:-auto}" = wayland ] && [ -z "$_usd_wl" ]; then
  echo "no live Wayland socket" >&2; exit 2
fi
if [ -n "$_usd_wl" ]; then
  _usd_backend=wayland
  export XDG_RUNTIME_DIR="$_usd_xdg_runtime"
  export WAYLAND_DISPLAY="$_usd_wl"
  unset DISPLAY
else
  _usd_backend=x11
  export DISPLAY="${DISPLAY:-:1}"
  unset WAYLAND_DISPLAY
fi
%s
]]):format(cmd)
end

local function need(tool, why)
  assert(oslo.run{ "sh", "-c", "command -v " .. tool, capture = true }.ok, why)
end

make.recipe{ name = "build", desc = "build the usdview binary",
             run = function() sh.cargo("build", "--bin", APP) end }
make.alias("b", "build")

make.recipe{ name = "compile", desc = "clean, then build", deps = { "clean", "build" } }
make.alias("c", "compile")

make.recipe{
  name = "run",
  desc = "launch usdview (fresh Wayland detection, host graphics on NixOS)",
  params = {
    { "--args", desc = "passed through to usdview, e.g. a USD scene path" },
    { "--profile", desc = "debug (default) or release" },
  },
  run = function(a)
    local profile = a.profile or "debug"
    assert(profile == "debug" or profile == "release", "--profile must be debug or release")
    if profile == "release" then sh.cargo("build", "--release", "--bin", APP)
    else sh.cargo("build", "--bin", APP) end
    sh.sh("-c", fresh_display_env(([[
set -- target/%s/%s %s
if [ ! -e /etc/NIXOS ]; then
  set -- nixVulkan "$@"
fi
exec env WGPU_VALIDATION="${USD_WGPU_VALIDATION:-${WGPU_VALIDATION:-0}}" \
  WGPU_DEBUG="${USD_WGPU_DEBUG:-${WGPU_DEBUG:-0}}" WINIT_UNIX_BACKEND=$_usd_backend "$@"
]]):format(profile, APP, a.args or "")))
  end,
}
make.alias("r", "run")

make.recipe{
  name = "build-example",
  desc = "build an example target",
  params = { { "--name", desc = "example target name" } },
  run = function(a)
    assert(type(a.name) == "string" and a.name:match("^[%w_%-]+$"), "provide --name EXAMPLE")
    sh.cargo("build", "--example", a.name)
  end,
}

make.recipe{
  name = "run-example",
  desc = "build and run an example with host graphics on NixOS",
  params = {
    { "--name", desc = "example target name" },
    { "--args", desc = "arguments passed through to the example" },
  },
  run = function(a)
    assert(type(a.name) == "string" and a.name:match("^[%w_%-]+$"), "provide --name EXAMPLE")
    sh.cargo("build", "--example", a.name)
    sh.sh("-c", fresh_display_env(([[
set -- target/debug/examples/%s %s
if [ ! -e /etc/NIXOS ]; then
  set -- nixVulkan "$@"
fi
exec env WGPU_VALIDATION="${USD_WGPU_VALIDATION:-${WGPU_VALIDATION:-0}}" \
  WGPU_DEBUG="${USD_WGPU_DEBUG:-${WGPU_DEBUG:-0}}" WINIT_UNIX_BACKEND=$_usd_backend "$@"
]]):format(a.name, a.args or "")))
  end,
}

make.recipe{
  name = "capture-reference",
  desc = "render a reference image with native usdrecord",
  params = { { "--args", desc = "arguments passed through to usdrecord" } },
  run = function(a) sh.sh("-c", "usdrecord " .. (a.args or "")) end,
}

make.recipe{ name = "serve-web", desc = "serve the usd_bevy web example in a browser (trunk, wasm32)",
             run = function() sh.sh("-c", "cd examples/web && NO_COLOR=true trunk serve --release --open") end }
make.recipe{ name = "build-web", desc = "build the web example's wasm bundle to examples/web/dist",
             run = function() sh.sh("-c", "cd examples/web && NO_COLOR=true trunk build --release") end }

make.recipe{ name = "test", desc = "test the usdview target",
             run = function() sh.cargo("test", "--bin", APP) end }
make.alias("t", "test")
make.recipe{ name = "test-all", desc = "run the full workspace all-target test suite",
             run = function() sh.cargo("test", "--workspace", "--all-targets") end }
make.recipe{ name = "test-native", desc = "check editor exports with native OpenUSD usdcat",
             run = function()
               sh.cargo("test", "--workspace", "--lib", "persistence::tests::native_export",
                        "--", "--ignored", "--nocapture")
               sh.cargo("test", "-p", "usd_bevy", "native_reference_custom_data_round_trip",
                        "--", "--ignored", "--nocapture")
             end }

make.recipe{
  name = "test-watchers",
  desc = "test native filesystem events with the file watcher enabled",
  run = function()
    sh.cargo("test", "-p", "usd_bevy", "--all-features", "asset::tests::native_",
             "--", "--ignored", "--test-threads=1", "--nocapture")
    sh.cargo("test", "-p", "usd_bevy", "--all-features", "editor::texture_watch::tests::native",
             "--", "--ignored", "--test-threads=1", "--nocapture")
  end,
}

make.recipe{ name = "check", desc = "cargo check on the usdview target",
             run = function() sh.cargo("check", "--bin", APP) end }
make.recipe{ name = "check-all", desc = "cargo check on the full workspace, all targets",
             run = function() sh.cargo("check", "--workspace", "--all-targets") end }

make.recipe{ name = "check-web", desc = "check the browser target on wasm32",
             run = function() sh.cargo("check", "-p", "usd_web", "--target", "wasm32-unknown-unknown") end }

make.recipe{
  name = "test-library",
  desc = "test the library without editor feature unification, then with all features",
  run = function()
    sh.cargo("test", "-p", "usd_bevy", "--no-default-features")
    sh.cargo("test", "-p", "usd_bevy", "--all-features")
  end,
}

make.recipe{ name = "test-assets", desc = "test the patched Bevy asset loader",
             run = function() sh.cargo("test", "-p", "bevy_asset", "--lib") end }

make.recipe{ name = "test-host", desc = "test the patched Mara Bevy host",
             run = function() sh.cargo("test", "-p", "mara_bevy", "--lib") end }

make.recipe{
  name = "test-capture-tools",
  desc = "test capture preflight, process cleanup and log failure detection",
  run = function()
    sh.bash("scripts/check_capture_preflight_tests.sh")
    sh.bash("scripts/check_capture_session.sh")
    sh.bash("scripts/check_viewer_log_tests.sh")
  end,
}

make.recipe{
  name = "test-ui",
  desc = "capture the native editor in an isolated Weston compositor",
  params = {
    { "--asset", desc = "scene path (default: flagship_showcase.usda)" },
    { "--output", desc = "new PNG path" },
  },
  deps = { "build" },
  run = function(a)
    assert(type(a.output) == "string", "provide --output NEW.png")
    sh.bash("scripts/capture_viewer_ui.sh", a.asset or "assets/flagship_showcase.usda", a.output)
  end,
}

make.recipe{
  name = "build-close-probe",
  desc = "build the X11 native-window-close probe",
  run = function()
    sh.sh("-c", "mkdir -p target && cc -Wall -Wextra -Werror scripts/send_window_close.c -o target/send_window_close $(pkg-config --cflags --libs x11)")
  end,
}

make.recipe{
  name = "test-native-close",
  desc = "exercise prompt, cancel or discard after an OS close request",
  params = {
    { "--output", desc = "new evidence directory" },
    { "--mode", desc = "prompt (default), cancel or discard" },
  },
  deps = { "build", "build-close-probe" },
  run = function(a)
    assert(type(a.output) == "string", "provide --output NEW_DIRECTORY")
    sh.bash("scripts/check_native_close.sh", a.output, a.mode or "prompt")
  end,
}

make.recipe{
  name = "test-transparency",
  desc = "capture and compare ordinary blending and OIT ordering",
  params = { { "--output", desc = "new directory for capture evidence" } },
  run = function(a)
    assert(type(a.output) == "string", "provide --output NEW_DIRECTORY")
    sh.bash("scripts/check_transparency_order.sh", a.output)
  end,
}

make.recipe{
  name = "test-render",
  desc = "capture WESL material, deformation, instancing and OIT fixtures",
  params = { { "--output", desc = "new directory for capture evidence" } },
  run = function(a)
    assert(type(a.output) == "string", "provide --output NEW_DIRECTORY")
    sh.bash("scripts/check_wesl_render.sh", a.output)
  end,
}

make.recipe{ name = "fmt", desc = "format the workspace crates (not vendor/)", run = function() fmt() end }

make.recipe{
  name = "harden",
  desc = "diff whitespace, fmt, no-default-features check/test, strict clippy, all-feature tests",
  run = function()
    sh.git("diff", "--check")
    fmt("--", "--check")
    sh.cargo("check", "--workspace", "--no-default-features")
    sh.cargo("test", "--workspace", "--no-default-features")
    sh.cargo("clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings")
    sh.cargo("test", "--workspace", "--all-targets", "--all-features")
  end,
}

make.recipe{ name = "bench", desc = "run benchmarks",
             run = function() sh.cargo("bench") end }

make.recipe{ name = "clean", desc = "remove Cargo build artifacts",
             run = function() sh.cargo("clean") end }

make.recipe{
  name = "docs",
  desc = "build the mdbook site into docs/, and commit it",
  run = function()
    need("mdbook", "mdbook is not installed; install it first")
    local top = oslo.sys.pwd()
    sh.mdbook("build", top .. "/book", "--dest-dir", top .. "/docs")
    sh.git("add", "--all")
    sh.git("commit", "-m", "docs: building website/mdbook")
  end,
}

make.recipe{
  name = "release",
  desc = "cut a version: --type patch | minor | major | M.m.p",
  params = { { "--type", desc = "patch | minor | major | M.m.p" } },
  run = function(a)
    need("git-rel", "git-rel is not installed; install it first")
    assert(type(a.type) == "string",
           "which release? make release --type patch|minor|major|M.m.p")
    sh.git("rel", a.type)
  end,
}
