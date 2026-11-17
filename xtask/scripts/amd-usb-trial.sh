#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# AMD fork (J6): a USB stick image for the GPU trial on the real PC, the
# same boot `amd-gpu-trial.sh` runs in a VM: `stela-amdgpu.efi` as the
# removable-media loader, the engine's test kernel with `--gpu-trial`,
# amdgpu's firmware bundle, and an empty boot log the boot fills
# (EFI/stela/boot.log), since a PC has no serial port to read.
#
#   xtask/scripts/amd-usb-trial.sh image       build .stela/usb/stela-amd-trial.img
#   xtask/scripts/amd-usb-trial.sh vm          boot the image in a VM (no card lent)
#   xtask/scripts/amd-usb-trial.sh log FILE    print a boot log copied off the stick
#
# Write the image to a USB stick whose contents may be lost (check the
# device with lsblk first):
#
#   sudo dd if=.stela/usb/stela-amd-trial.img of=/dev/sdX bs=4M conv=fsync status=progress
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
efi=$root/target/boot/x86_64-unknown-uefi/release/stela-amdgpu.efi
image=$root/.stela/usb/stela-amd-trial.img
bundle=$root/.stela/vm/amdgpu-firmware
kernel=$root/.stela/vm/gpu-trial/volume/EFI/stela/source.bin
options='--system --load 0x800004000 --entry 0x800004000 --gpu-trial'
# The ESP type GUID.
esp=C12A7328-F81F-11D2-BA4B-00A0C93EC93B

build_image() {
  [[ -f $efi ]] || { echo "amd-usb-trial: build the image first (amd-gpu-trial.sh build)" >&2; exit 1; }
  [[ -f $kernel ]] || { echo "amd-usb-trial: run one VM trial first, which writes the test kernel" >&2; exit 1; }
  [[ -f $bundle/stela/firmware/bundle.json ]] || python3 -I "$root/xtask/scripts/amd-firmware-bundle.py"
  mkdir -p "$(dirname "$image")"
  rm -f "$image"
  truncate -s 256M "$image"
  printf 'label: gpt\nstart=2048, type=%s, name="STELA"\n' "$esp" | sfdisk -q "$image"
  local start sectors
  read -r start sectors < <(sfdisk -d "$image" | sed -n 's/.*start= *\([0-9]*\), size= *\([0-9]*\).*/\1 \2/p')
  mkfs.fat -F 32 -n STELA --offset "$start" "$image" $((sectors / 2)) >/dev/null
  local at="$image@@$((start * 512))"
  mmd -i "$at" ::/EFI ::/EFI/BOOT ::/EFI/stela ::/stela
  mcopy -i "$at" "$efi" ::/EFI/BOOT/BOOTX64.EFI
  mcopy -i "$at" "$kernel" ::/EFI/stela/source.bin
  printf '%s\n' "$options" >"$image.cfg"
  mcopy -i "$at" "$image.cfg" ::/EFI/stela/stela.cfg
  rm -f "$image.cfg"
  # The log Stela writes from its first line to its last.
  head -c $((4 << 20)) /dev/zero >"$image.log"
  mcopy -i "$at" "$image.log" ::/EFI/stela/boot.log
  rm -f "$image.log"
  mcopy -s -i "$at" "$bundle/stela/firmware" ::/stela/
  echo "amd-usb-trial: $image ($(du -h "$image" | cut -f1)): write it to an empty USB stick"
}

# Boot the image in a VM, with no card lent: the stick boots, its log fills.
vm() {
  local out=$root/.stela/vm/usb-trial
  rm -rf "$out"
  cd "$root"
  cargo xtask boot run --kvm --usb-image "$image" --display none --timeout 120 --output "$out" ||
    true
  grep -aE "BOOT LOG|GPU TRIAL|FIRMWARE BUNDLE USE|STOP" "$out/serial.txt" | head -20
}

case ${1:-} in
  image) build_image ;;
  vm) vm ;;
  log)
    file=${2:?the boot log copied off the stick}
    cd "$root"
    cargo run -q -p stela-cli -- boot-log "$file" 2>/dev/null || strings "$file" | tail -200
    ;;
  *) echo "usage: $0 image|vm|log FILE" >&2; exit 2 ;;
esac
