#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Lends the RDNA 4 Radeon to N Stela VM boots in a row (J1): each trial takes
# the card from amdgpu, boots `stela.efi` under QEMU/KVM with it, and gives
# it back, so the run measures how the card survives being reset again and
# again. Every trial's run report, serial log and the card's state go to
# .stela/vm/; the desktop stops for each trial and comes back after it.
#
# The desktop (and every graphical session on the card) stops, so start it
# detached, as root, naming the user whose build and cargo it uses:
#
#   sudo systemd-run --unit=stela-amd-vm --collect \
#     --setenv=STELA_VFIO_USER=$USER "$PWD/xtask/scripts/amd-vm-trials.sh" 3
#
# then read .stela/vm/report.txt once the desktop is back.
set -uo pipefail

trials=${1:-3}
root=$(cd "$(dirname "$0")/../.." && pwd)
user=${STELA_VFIO_USER:?set STELA_VFIO_USER to the user who owns the checkout}
home=$(getent passwd "$user" | cut -d: -f6)
card=${STELA_VFIO_GPU:-0000:07:00.0}
out=$root/.stela/vm
report=$out/report.txt
mkdir -p "$out"
chown "$user:" "$out"

state() {
  local driver=none
  [[ -e /sys/bus/pci/devices/$card/driver ]] && driver=$(basename "$(readlink /sys/bus/pci/devices/$card/driver)")
  echo "  $1: driver=$driver, reset_method='$(cat /sys/bus/pci/devices/$card/reset_method 2>/dev/null)'"
}

{
  echo "Stela VM trials with $card lent, $(date -Is)"
  state "before"
} >"$report"

for trial in $(seq 1 "$trials"); do
  dir=$out/trial-$trial
  rm -rf "$dir"
  start=$(date +%s)
  dmesg_from=$(dmesg | wc -l)
  "$root/xtask/scripts/vfio-gpu.sh" run --stop-desktop --seconds 900 -- \
    bash -c "cd '$root' && '$home/.cargo/bin/cargo' xtask boot run --kvm --vfio-gpu '$card' \
      --display none --timeout 120 --output '$dir'"
  code=$?
  {
    echo "trial $trial: exit=$code in $(($(date +%s) - start)) s"
    state "after"
    grep -iE "GPU|Radeon|Navi|AMD [0-9a-f]{4}|amdgpu" "$dir/serial.txt" 2>/dev/null | head -20 | sed 's/^/    serial: /'
    dmesg | tail -n +"$((dmesg_from + 1))" | grep -iE "amdgpu|vfio|0000:07:00" | tail -15 | sed 's/^/    dmesg: /'
  } >>"$report"
  # The card is amdgpu's again before the next trial takes it.
  sleep 20
done
state "end" >>"$report"
chown -R "$user:" "$out"
