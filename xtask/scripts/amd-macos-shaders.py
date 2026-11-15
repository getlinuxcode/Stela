#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Compile macOS's own Metal shaders with Stela's compiler for an AMD family.

The metallibs come from `cargo xtask gpu metal-suite coverage`, which copies
them from a Mac into DIR/system. Each metallib is split into its function
modules (0x0B17C0DE wrappers, as coverage.rs splits them), and each module is
compiled by stela_metal_corpus_amd, which reports every entry point as ok or
refused. No GPU work is involved.

    xtask/scripts/amd-macos-shaders.py --tool PATH/stela_metal_corpus_amd \
        [--gpu gfx1201] [--dir .stela/metal-suite] [--all] [--out FILE]

With --sm SM and the NVIDIA stela_metal_corpus instead, the same corpus
compiles with NAK's options: what both refuse comes from the shared AIR
frontend, not from the AMD target.

Without --all, only the libraries a first boot draws with: WindowServer
(SkyLight), CoreAnimation (QuartzCore), the setup assistant, CoreUI,
CoreImage and Metal. Prints a summary and writes every outcome as JSON.
"""
import argparse
import collections
import concurrent.futures
import json
import os
import re
import struct
import subprocess
import sys
import tempfile

BOOT = re.compile(
    r"/(SkyLight|QuartzCore|SetupAssistantSupportUI|CoreUI|CoreImage|Metal)\.framework/"
)
MAGIC = b"\xde\xc0\x17\x0b"
LINE = re.compile(r"^(?P<module>\S+) (?P<entry>\S+): (?P<outcome>ok.*|refused.*|reflection refused.*)$")


def modules(data):
    """The function modules of a metallib."""
    out, at = [], 0
    while True:
        start = data.find(MAGIC, at)
        if start < 0:
            return out
        if start + 16 <= len(data):
            offset, size = struct.unpack_from("<II", data, start + 8)
            end = start + offset + size
            if end <= len(data):
                out.append(data[start:end])
        at = start + 4


def compile_module(tool, target, path):
    run = subprocess.run([tool, *target, path], capture_output=True, text=True)
    results = []
    for line in run.stdout.splitlines():
        match = LINE.match(line)
        if match:
            results.append((match["entry"], match["outcome"]))
    if run.returncode not in (0, 1) or (run.returncode == 1 and not results):
        tail = (run.stdout + run.stderr).strip().splitlines()[-1:] or ["no output"]
        results.append(("<module>", f"refused (crash {run.returncode}): {tail[0]}"))
    return results


def reason(outcome):
    """A refusal's reason without the names that vary per function."""
    text = re.sub(r"^(reflection )?refused \(\d+\): ", "", outcome)
    text = re.sub(r"_Z\w+|'[^']*'|\b\d+\b", "#", text)
    return text[:120]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tool", required=True)
    parser.add_argument("--gpu", default="gfx1201")
    parser.add_argument("--sm", help="an NVIDIA SM, for the NVIDIA tool")
    parser.add_argument("--dir", default=".stela/metal-suite")
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--out")
    parser.add_argument("--jobs", type=int, default=os.cpu_count())
    args = parser.parse_args()

    target = ["--sm", args.sm] if args.sm else ["--gpu", args.gpu]
    label = f"sm{args.sm}" if args.sm else args.gpu
    system = os.path.join(args.dir, "system")
    libraries = sorted(
        os.path.join(root, name)
        for root, _, names in os.walk(system)
        for name in names
        if name.endswith(".metallib") and "/AGXMetal" not in root
    )
    if not args.all:
        libraries = [path for path in libraries if BOOT.search(path)]
    if not libraries:
        sys.exit(f"no metallib under {system}; run metal-suite coverage first")

    outcomes = {}
    with tempfile.TemporaryDirectory() as scratch:
        jobs = []
        for index, library in enumerate(libraries):
            with open(library, "rb") as file:
                parts = modules(file.read())
            for part, module in enumerate(parts):
                path = os.path.join(scratch, f"{index}.{part}.air")
                with open(path, "wb") as file:
                    file.write(module)
                jobs.append((os.path.relpath(library, system), part, path))
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            futures = {
                pool.submit(compile_module, args.tool, target, path): (library, part)
                for library, part, path in jobs
            }
            for future in concurrent.futures.as_completed(futures):
                library, part = futures[future]
                # Binary archives repeat entry names across their modules.
                for entry, outcome in future.result():
                    outcomes.setdefault(library, {})[f"{part}:{entry}"] = outcome

    reasons = collections.Counter()
    print(f"{label}: {len(libraries)} libraries, {sum(map(len, outcomes.values()))} entry points")
    for library in sorted(outcomes):
        entries = outcomes[library]
        refused = {e: o for e, o in entries.items() if not o.startswith("ok")}
        reasons.update(reason(o) for o in refused.values())
        print(f"  {len(entries) - len(refused):5} ok {len(refused):5} refused  {library}")
    print("refusals by reason:")
    for text, count in reasons.most_common():
        print(f"  {count:5}  {text}")
    if args.out:
        with open(args.out, "w") as file:
            json.dump({"target": label, "outcomes": outcomes}, file, indent=1, sort_keys=True)


if __name__ == "__main__":
    main()
