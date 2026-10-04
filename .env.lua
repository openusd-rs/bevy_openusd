-- bevy_openusd's directory environment. Loaded when you `cd` here, unloaded when you leave.

-- Exports NVIDIA_VERSION before nix_develop() so flake.nix picks the matching
-- nixVulkan/nixGL wrapper; without it the flake falls back to the Intel ones.
local function busy_wait(seconds)
  local start = os.clock()
  while os.clock() - start < seconds do end
end

local function detect_nvidia_version()
  local has_nvidia = false
  for _, path in ipairs(oslo.fs.glob("/sys/bus/pci/devices/*/vendor")) do
    local vendor = oslo.fs.read(path)
    if vendor and vendor:match("^%s*0x10de%s*$") then
      has_nvidia = true
      break
    end
  end
  if not has_nvidia then
    return nil
  end
  for _ = 1, 5 do
    local ver = oslo.fs.read("/proc/driver/nvidia/version")
    if ver then
      local version = ver:match("%s%s(%d[%d%.]*)%s%sRelease")
      if version then
        return version
      end
    end
    busy_wait(0.3)
  end
  return nil
end

local nvidia_version = detect_nvidia_version()
if nvidia_version then
  oslo.env.set("NVIDIA_VERSION", nvidia_version)
  -- PRIME render-offload onto the discrete GPU.
  oslo.env.set("__NV_PRIME_RENDER_OFFLOAD", "1")
  oslo.env.set("__NV_PRIME_RENDER_OFFLOAD_PROVIDER", "NVIDIA-G0")
  oslo.env.set("__GLX_VENDOR_LIBRARY_NAME", "nvidia")
  oslo.env.set("__VK_LAYER_NV_optimus", "NVIDIA_only")
end

oslo.direnv.nix_develop({ impure = true })

oslo.direnv.path_add("./target/debug")
oslo.direnv.path_add("./target/release")

oslo.env.set("TOP_HEAD", oslo.sys.pwd())

oslo.env.set_alias("_b", "make build")
oslo.env.set_alias("_c", "make compile")
oslo.env.set_alias("_r", "make run")
oslo.env.set_alias("_t", "make test")
