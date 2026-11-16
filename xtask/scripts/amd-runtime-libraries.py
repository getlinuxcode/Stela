#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Compile the Metal suite's run-time libraries ahead of time, on a Mac.

The suite compiles its *_runtime_* libraries from MSL source as a case runs,
with the MTLCompileOptions the plan names (`library_named` in runner.m).
macOS gives no Metal library a run-time compile makes as bytes, so this
compiles each with `xcrun metal` and the command-line flags of the same
options into DIR/plan/lib-runtime/NAME.metallib, which `stela-amd-host
--suite` loads for those libraries. A library whose options have no flag
(conversion rounding, required threads, dynamic linking) is left out.

    xtask/scripts/amd-runtime-libraries.py --host USER@MAC [--dir .stela/metal-suite]
"""
import argparse
import json
import os
import shlex
import subprocess
import sys
import tarfile
import io

REMOTE = "stela-runtime-libraries"


def language_flag(version):
    """-std for an MSL version: macOS-only names before Metal 3."""
    major = int(version.split(".")[0])
    return f"-std=metal{version}" if major >= 3 else f"-std=macos-metal{version}"


def flags(options):
    """The `metal` flags of MTLCompileOptions, or None when one has none."""
    unsupported = {"conversion_rounding", "required_threads", "libraries", "install_name",
                   "symbol_visibility", "allow_undefined"}
    if unsupported & set(options):
        return None
    out = []
    if "language" in options:
        out.append(language_flag(options["language"]))
    if "fast_math" in options:
        out.append("-ffast-math" if options["fast_math"] else "-fno-fast-math")
    if "math_mode" in options or "math_functions" in options:
        # As the runner: a mode or function precision left out is fast.
        out.append(f"-fmetal-math-mode={options.get('math_mode', 'fast')}")
        out.append(f"-fmetal-math-fp32-functions={options.get('math_functions', 'fast')}")
    for name, value in sorted(options.get("macros", {}).items()):
        out.append(f"-D{name}={value}")
    if options.get("preserve_invariance"):
        out.append("-fpreserve-invariance")
    if options.get("logging"):
        out.append("-fmetal-enable-logging")
    if options.get("optimization") == "size":
        out.append("-Os")
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--host", required=True)
    parser.add_argument("--dir", default=".stela/metal-suite")
    args = parser.parse_args()
    plan_dir = os.path.join(args.dir, "plan")
    with open(os.path.join(plan_dir, "plan.json")) as file:
        plan = json.load(file)
    runtime = {name: options for name, options in plan["libraries"].items()
               if isinstance(options, dict)}

    script = ["set -u", "export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer",
              f"cd ~/{REMOTE}", "mkdir -p out"]
    sources = set()
    skipped = []
    for name, options in sorted(runtime.items()):
        command = flags(options)
        if command is None:
            skipped.append(name)
            continue
        source = options["source"]
        sources.add(source)
        script.append(
            "xcrun -sdk macosx metal " + " ".join(shlex.quote(f) for f in command)
            + f" -o out/{name}.metallib {shlex.quote(source)} 2> out/{name}.log"
            + f" || echo 'failed: {name}'"
        )
    script.append("tar -cf - out")

    archive = io.BytesIO()
    with tarfile.open(fileobj=archive, mode="w") as tar:
        for source in sorted(sources):
            tar.add(os.path.join(plan_dir, source), arcname=source)
        body = "\n".join(script).encode() + b"\n"
        info = tarfile.TarInfo("build.sh")
        info.size = len(body)
        tar.addfile(info, io.BytesIO(body))
    remote = (f"rm -rf ~/{REMOTE} && mkdir -p ~/{REMOTE} && tar -C ~/{REMOTE} -xf - "
              f"&& bash ~/{REMOTE}/build.sh")
    run = subprocess.run(["ssh", "-o", "BatchMode=yes", args.host, remote],
                         input=archive.getvalue(), capture_output=True)
    if run.returncode != 0:
        sys.exit(run.stderr.decode(errors="replace"))
    out = os.path.join(plan_dir, "lib-runtime")
    os.makedirs(out, exist_ok=True)
    built = []
    with tarfile.open(fileobj=io.BytesIO(run.stdout)) as tar:
        for member in tar.getmembers():
            name = os.path.basename(member.name)
            if member.isfile() and name.endswith((".metallib", ".log")):
                data = tar.extractfile(member).read()
                with open(os.path.join(out, name), "wb") as file:
                    file.write(data)
                if name.endswith(".metallib"):
                    built.append(name)
    print(f"{len(built)} run-time libraries in {out}; "
          f"{len(skipped)} left out (options without flags): {', '.join(skipped)}")
    for line in run.stderr.decode(errors="replace").splitlines():
        print(line)


if __name__ == "__main__":
    main()
