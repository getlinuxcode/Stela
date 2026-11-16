#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Build a USB-style firmware bundle holding amdgpu's pinned files (AMD fork).

Stela's boot reads `stela/firmware/bundle.json` from its volume and hands a
driver each pinned file it needs, found by digest
(`crates/stela-loader/src/host_firmware/bundle.rs`). A VM trial has no USB:
this writes the bundle for the Radeon's recipes alone
(`crates/stela-loader/src/host_firmware/recipes/amdgpu.rs`), downloaded from
linux-firmware at their pinned revision and checked against each pin, into

    .stela/vm/amdgpu-firmware/stela/firmware/

which `amd-gpu-trial.sh` puts on the VM's boot volume
(STELA_BOOT_VOLUME_EXTRA). Downloads are cached in .stela/firmware-cache/.

    python3 -I xtask/scripts/amd-firmware-bundle.py
"""
import hashlib
import json
import os
import re
import shutil
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
RECIPES = os.path.join(ROOT, "crates/stela-loader/src/host_firmware/recipes/amdgpu.rs")
REVISIONS = os.path.join(ROOT, "crates/stela-loader/src/host_firmware/recipes.rs")
OUT = os.path.join(ROOT, ".stela/vm/amdgpu-firmware")
CACHE = os.path.join(ROOT, ".stela/firmware-cache")
SOURCE = "https://gitlab.com/kernel-firmware/linux-firmware"
KERNEL_ORG = "https://git.kernel.org/pub/scm/linux/kernel/git/firmware/linux-firmware.git"


def revision():
    text = open(REVISIONS).read()
    return re.search(r'pub const REALTEK_REVISION: &str = "([0-9a-f]{40})";', text).group(1)


def artifacts():
    text = open(RECIPES).read()
    files = re.findall(r'amdgpu\(\s*"([^"]+)",\s*(\d+),\s*"([0-9a-f]{64})",?\s*\)', text)
    license = re.search(
        r'member: "(LICENSES/[^"]+)",\s*length: (\d+),\s*sha256: "([0-9a-f]{64})"', text
    )
    if not files or not license:
        sys.exit("amd-firmware-bundle: cannot read the recipes")
    as_dict = lambda m, n, s: {"member": m, "length": int(n), "sha256": s}
    return [as_dict(*f) for f in files], as_dict(*license.groups())


def fetch(artifact, rev):
    cached = os.path.join(CACHE, artifact["sha256"])
    if os.path.exists(cached):
        data = open(cached, "rb").read()
    else:
        url = f"{KERNEL_ORG}/plain/{artifact['member']}?id={rev}"
        # kernel.org refuses urllib's own User-Agent.
        request = urllib.request.Request(url, headers={"User-Agent": "stela-firmware-bundle"})
        with urllib.request.urlopen(request, timeout=60) as response:
            data = response.read()
    if len(data) != artifact["length"] or hashlib.sha256(data).hexdigest() != artifact["sha256"]:
        sys.exit(f"amd-firmware-bundle: {artifact['member']} differs from its pin")
    os.makedirs(CACHE, exist_ok=True)
    with open(cached, "wb") as file:
        file.write(data)
    return data


def main():
    rev = revision()
    files, license = artifacts()
    directory = os.path.join(OUT, "stela/firmware")
    shutil.rmtree(OUT, ignore_errors=True)
    os.makedirs(os.path.join(directory, "licenses"))
    notice_path = f"licenses/{license['sha256'][:32]}.txt"
    with open(os.path.join(directory, notice_path), "wb") as file:
        file.write(fetch(license, rev))
    objects = []
    for artifact in files:
        path = f"{artifact['sha256'][:32]}.bin"
        with open(os.path.join(directory, path), "wb") as file:
            file.write(fetch(artifact, rev))
        objects.append({"path": path, "sha256": artifact["sha256"], "length": artifact["length"],
                        "source": SOURCE, "revision": rev, "member": artifact["member"],
                        "license": notice_path})
    notices = [{"path": notice_path, "sha256": license["sha256"], "length": license["length"],
                "source": SOURCE, "revision": rev, "member": license["member"]}]
    with open(os.path.join(directory, "bundle.json"), "w") as file:
        json.dump({"schema": 1, "objects": objects, "notices": notices}, file, indent=2)
    total = sum(a["length"] for a in files)
    print(f"{len(files)} amdgpu files, {total} bytes, in {os.path.relpath(OUT, ROOT)}")


if __name__ == "__main__":
    main()
