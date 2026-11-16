#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Read amdgpu's Kbuild with GNU Make, for Stela's configuration of it.

amdgpu's makefiles (amdgpu, display, pm, ras) use more of Make than the
subset `crates/stela-driver-gpu/build/kbuild.rs` reads: variable include
paths, `addprefix` and `foreach` lists, per-object flags. This evaluates
them with Make itself, under the configuration below, and writes what the
port compiles to `crates/stela-driver-gpu/build/amdgpu-kbuild.json`:

- `objects`: the `amdgpu-y` composite, relative to amdgpu's directory;
- `ccflags`: amdgpu's `ccflags-y` and `subdir-ccflags-y`, Kbuild's paths
  made relative to `drivers/gpu/drm/amd`;
- `object_flags`: each object's own `CFLAGS_<object>.o` (the display
  core's floating-point units take `$(CC_FLAGS_FPU)`), and
  `removed_flags`: its `CFLAGS_REMOVE_<object>.o`.

Run it after the Linux pin or this configuration changes:

    python3 -I xtask/scripts/amd-kbuild.py
"""
import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
AMD = os.path.join(ROOT, "third_party/linux/drivers/gpu/drm/amd")
OUT = os.path.join(ROOT, "crates/stela-driver-gpu/build/amdgpu-kbuild.json")

# Stela's amdgpu: RDNA 4 desktop cards, with the display core (DCN needs
# its floating-point code). As nouveau, under Stela's Kconfig, which has no
# ACPI. No GCN 1/2 (SI, CIK), no compute API (KFD), no userptr (HMM), no
# audio coprocessor (ACP), no image signal processor, no debugfs, procfs,
# perf or 32-bit ioctls.
CONFIG = {
    "CONFIG_DRM_AMDGPU": "y",
    "CONFIG_DRM_AMD_DC": "y",
    "CONFIG_DRM_AMD_DC_FP": "y",
    "CONFIG_CC_IS_CLANG": "y",
    "CONFIG_DRM_AMD_SECURE_DISPLAY": "",
    "CONFIG_DEBUG_FS": "",
    "CONFIG_HSA_AMD": "",
}
# What the sources see of it: Stela's Kconfig header lacks amdgpu's options.
DEFINES = ["-DCONFIG_DRM_AMDGPU=1", "-DCONFIG_DRM_AMD_DC=1", "-DCONFIG_DRM_AMD_DC_FP=1"]
# Flags Linux's top Makefile gives every Clang build, which amdgpu needs:
# its unions of flexible arrays (amdgv_sriovmsg.h) sit inside structures,
# and mempool's anonymous structures are Microsoft's extension.
LINUX_CLANG = ["-Wno-gnu-variable-sized-type-not-at-end", "-fms-extensions", "-Wno-microsoft-anon-tag"]
# arch/x86/Makefile's floating-point flags under Clang (GCC adds
# -mhard-float), which the display core's floating-point units take.
FPU = "-msse -msse2"
NO_FPU = "-mno-sse -mno-mmx -mno-sse2 -mno-3dnow -mno-avx"
PREFIX = "drivers/gpu/drm/amd/amdgpu/"

DRIVER = r"""
src := {src}
obj := drivers/gpu/drm/amd/amdgpu
srctree := {srctree}
CC_FLAGS_FPU := {fpu}
CC_FLAGS_NO_FPU := {no_fpu}
cc-option = $(1)
cc-disable-warning = -Wno-$(strip $(1))
{config}
include $(src)/Makefile

objects := $(sort $(amdgpu-y))
print:
	$(file >{out},objects=$(objects))
	$(file >>{out},ccflags=$(ccflags-y) $(subdir-ccflags-y))
	$(foreach v,$(filter CFLAGS_%,$(.VARIABLES)),$(file >>{out},$(v)=$($(v))))
"""


def relative(flag):
    """A `-I` path relative to drivers/gpu/drm/amd, other flags as they are."""
    if flag.startswith("-I"):
        path = os.path.normpath(flag[2:])
        return "-I" + os.path.relpath(path, AMD)
    return flag


def main():
    with tempfile.TemporaryDirectory() as scratch:
        out = os.path.join(scratch, "values")
        makefile = os.path.join(scratch, "Makefile")
        config = "\n".join(f"{name} := {value}" for name, value in CONFIG.items())
        with open(makefile, "w") as file:
            file.write(DRIVER.format(src=os.path.join(AMD, "amdgpu"),
                                     srctree=os.path.join(ROOT, "third_party/linux"),
                                     fpu=FPU, no_fpu=NO_FPU, config=config, out=out))
        run = subprocess.run(["make", "-s", "-f", makefile, "print"], cwd=scratch,
                             capture_output=True, text=True)
        if run.returncode != 0:
            sys.exit(run.stderr)
        values = {}
        with open(out) as file:
            for line in file:
                name, _, value = line.rstrip("\n").partition("=")
                values[name] = value
    objects = values.pop("objects").split()
    ccflags = list(DEFINES + LINUX_CLANG)
    for flag in values.pop("ccflags").split():
        flag = relative(flag)
        if flag not in ccflags:
            ccflags.append(flag)
    object_flags, removed = {}, {}
    for name, value in sorted(values.items()):
        target = removed if name.startswith("CFLAGS_REMOVE_") else object_flags
        stem = name.removeprefix("CFLAGS_REMOVE_").removeprefix("CFLAGS_")
        stem = stem.removeprefix(PREFIX)
        if value.split():
            target[stem] = [relative(flag) for flag in value.split()]
    result = {
        "config": CONFIG,
        "objects": objects,
        "ccflags": ccflags,
        "object_flags": object_flags,
        "removed_flags": removed,
    }
    with open(OUT, "w") as file:
        json.dump(result, file, indent=1)
        file.write("\n")
    fpu = sum(1 for flags in object_flags.values() if "-msse2" in flags)
    print(f"{len(objects)} objects, {fpu} with floating point, in {os.path.relpath(OUT, ROOT)}")


if __name__ == "__main__":
    main()
