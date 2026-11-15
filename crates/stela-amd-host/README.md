# stela-amd-host

Qualifies an AMD GPU and its RADV driver for Stela's Metal device on a Linux
host, before a boot image carries amdgpu or RADV.

It surveys every Vulkan device as the native backend would, then for each
Radeon reads the graphics core and PCIe link from sysfs, admits it through
the AMD Metal route, and prints the Metal profile Stela would derive: GPU
families and every missing capability with the layer that lacks it.

Shader features are assumed complete (the AIR compiler for RADV is not built
here), so the report covers what RADV and Stela's Metal layer allow.

## Run

Linux 6.12 or newer with amdgpu, and a recent Mesa with RADV:

```sh
cargo run -p stela-amd-host --release
# Only RADV, when other Vulkan drivers are installed:
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.x86_64.json \
    cargo run -p stela-amd-host --release
```

The exit status is 0 when an AMD GPU qualifies.
