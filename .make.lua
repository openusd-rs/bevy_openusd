-- bevy_openusd's build, as recipes.
--
--   make            the recipes, with what each of them says it does
--   make build      the usdview binary
--   make run        usdview, through nixVulkan
--   make test       the usdview tests
--
-- At an oslo prompt in this directory `make` is enough; everywhere else it is `oslo make`.
-- The toolchain and the `nixVulkan` wrapper come from `.env.lua`'s dev shell.

local make = oslo.make

local APP = "usdview"

-- Picks a live Wayland socket per launch, else falls back to X11, then runs `cmd`.
local function fresh_display_env(cmd)
  return ([[
_usd_xdg_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
_usd_wl=""
if [ -n "${WAYLAND_DISPLAY:-}" ] && [ -S "$_usd_xdg_runtime/$WAYLAND_DISPLAY" ]; then
  _usd_wl="$WAYLAND_DISPLAY"
elif [ -S "$_usd_xdg_runtime/wayland-0" ]; then
  _usd_wl="wayland-0"
else
  _usd_wl=$(ls -t "$_usd_xdg_runtime"/wayland-* 2>/dev/null | grep -v '\.lock$' | head -n1 | xargs -r basename)
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
  desc = "launch usdview (fresh Wayland detection, nixVulkan wrapper)",
  params = { { "--args", desc = "passed through to usdview, e.g. a USD scene path" } },
  deps = { "build" },
  run = function(a)
    sh.sh("-c", fresh_display_env(
      ("env WINIT_UNIX_BACKEND=$_usd_backend nixVulkan target/debug/%s %s"):format(APP, a.args or "")))
  end,
}
make.alias("r", "run")

make.recipe{
  name = "capture-reference",
  desc = "render a reference image with native usdrecord",
  params = { { "--args", desc = "arguments passed through to usdrecord" } },
  run = function(a) sh.sh("-c", "usdrecord " .. (a.args or "")) end,
}

make.recipe{ name = "serve-web", desc = "serve the egui_mara UI in a browser (trunk, wasm32)",
             run = function() sh.sh("-c", "cd api_crates/web && trunk serve --open") end }
make.recipe{ name = "build-web", desc = "build the wasm bundle to api_crates/web/dist",
             run = function() sh.sh("-c", "cd api_crates/web && trunk build --release") end }

make.recipe{ name = "test", desc = "test the usdview target",
             run = function() sh.cargo("test", "--bin", APP) end }
make.alias("t", "test")
make.recipe{ name = "test-all", desc = "run the full workspace all-target test suite",
             run = function() sh.cargo("test", "--workspace", "--all-targets") end }
make.recipe{ name = "test-native", desc = "check editor exports with native OpenUSD usdcat",
             run = function()
               sh.cargo("test", "--workspace", "--lib", "persistence::tests::native_export",
                        "--", "--ignored", "--nocapture")
             end }

make.recipe{ name = "check", desc = "cargo check on the usdview target",
             run = function() sh.cargo("check", "--bin", APP) end }
make.recipe{ name = "check-all", desc = "cargo check on the full workspace, all targets",
             run = function() sh.cargo("check", "--workspace", "--all-targets") end }

make.recipe{
  name = "harden",
  desc = "diff whitespace, fmt, no-default-features check/test, strict clippy, all-feature tests",
  run = function()
    sh.git("diff", "--check")
    sh.cargo("fmt", "--all", "--", "--check")
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
