#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# AMD fork (J6): lends the RDNA 4 Radeon to one Stela VM boot whose image
# links amdgpu (`--features amdgpu`) and whose options start the GPU without
# macOS (`--gpu-trial`), so amdgpu probes the card. The serial log, the run
# report and the card's state go to .stela/vm/gpu-trial/, with the lines
# that matter in .stela/vm/gpu-trial/report.txt.
#
# Build the image first, as yourself (the desktop stays up):
#
#   xtask/scripts/amd-gpu-trial.sh build
#
# then run the trial detached, as root, since the desktop stops:
#
#   sudo systemd-run --unit=stela-amd-gpu --collect \
#     --setenv=STELA_VFIO_USER=$USER "$PWD/xtask/scripts/amd-gpu-trial.sh" run
#
# The desktop comes back after it; read .stela/vm/gpu-trial/report.txt.
set -uo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
image=$root/target/boot/x86_64-unknown-uefi/release/stela-amdgpu.efi
out=$root/.stela/vm/gpu-trial
card=${STELA_VFIO_GPU:-0000:07:00.0}

build() {
  cd "$root" || exit 1
  # The engine's test kernel, with the GPU trial added to its options.
  cargo build --offline --release --manifest-path crates/stela-boot/Cargo.toml \
    --target x86_64-unknown-uefi --target-dir target/boot \
    --features diagnostics,amdgpu || exit 1
  llvm-objcopy --strip-debug target/boot/x86_64-unknown-uefi/release/stela.efi "$image" 2>/dev/null ||
    cp target/boot/x86_64-unknown-uefi/release/stela.efi "$image"
  echo "amd-gpu-trial: $image"
}

state() {
  local driver=none
  [[ -e /sys/bus/pci/devices/$card/driver ]] && driver=$(basename "$(readlink /sys/bus/pci/devices/$card/driver)")
  echo "  $1: driver=$driver"
}

run() {
  user=${STELA_VFIO_USER:?set STELA_VFIO_USER to the user who owns the checkout}
  home=$(getent passwd "$user" | cut -d: -f6)
  [[ -f $image ]] || { echo "amd-gpu-trial: build the image first ($0 build)" >&2; exit 1; }
  rm -rf "$out"
  mkdir -p "$(dirname "$out")"
  report=$(dirname "$out")/gpu-trial-report.txt
  {
    echo "Stela GPU trial with $card lent, $(date -Is)"
    state before
  } >"$report"
  start=$(date +%s)
  dmesg_from=$(dmesg | wc -l)
  "$root/xtask/scripts/vfio-gpu.sh" run --stop-desktop --seconds 900 -- \
    bash -c "cd '$root' && '$home/.cargo/bin/cargo' xtask boot run --kvm --vfio-gpu '$card' \
      --image '$image' --display none --timeout 300 --output '$out' \
      --append '--system --load 0x800004000 --entry 0x800004000 --gpu-trial'"
  code=$?
  {
    echo "trial: exit=$code in $(($(date +%s) - start)) s"
    state after
    echo "  serial, GPU lines:"
    grep -aiE "GPU|amdgpu|Radeon|Navi|drm|firmware|psp|smu|gfx|dcn|discovery" "$out/serial.txt" 2>/dev/null |
      head -200 | sed 's/^/    /'
    echo "  host dmesg after:"
    dmesg | tail -n +"$((dmesg_from + 1))" | grep -iE "amdgpu|vfio|$card" | tail -20 | sed 's/^/    /'
  } >>"$report"
  mv "$report" "$out/report.txt" 2>/dev/null || true
  chown -R "$user:" "$(dirname "$out")"
}

case ${1:-} in
  build) build ;;
  run) run ;;
  *) echo "usage: $0 build|run" >&2; exit 2 ;;
esac
