#!/bin/bash
# Lend a host's GPU to one `cargo xtask boot run --kvm --vfio-gpu` and take it
# back. Linux keeps no VFIO configuration between runs: the card is bound to
# vfio-pci for the command's life and returned to its driver afterwards.
#
#   vfio-gpu.sh status
#   sudo vfio-gpu.sh run [--stop-desktop [--autologin]] [--seconds N] -- COMMAND...
#
# COMMAND runs as the sudo caller, with memory locking unlimited so VFIO can
# pin guest RAM. --stop-desktop stops the display manager first and restarts
# it afterwards: a card that drives the only display cannot be unbound, and
# every graphical session on it ends. Start such a run detached so it survives
# them; it always gives the desktop back, within --seconds at the latest:
#
#   sudo systemd-run --unit=stela-vfio --collect xtask/scripts/vfio-gpu.sh \
#     run --stop-desktop --seconds 1800 -- COMMAND...
#
# --autologin (AMD fork): the restarted desktop logs the caller back in
# without a password, once. A LightDM drop-in names them for that one start
# and is removed as soon as LightDM has read it, so the session (and a tmux
# session or agent in it) comes back as it would after a reboot.
#
# STELA_VFIO_GPU names the card, by default the first NVIDIA display function,
# else the first RDNA 4 Radeon (Navi 48 0x7550, Navi 44 0x7590; an APU's own
# AMD display function is never chosen). A Radeon's framebuffer consoles are
# released before amdgpu lets it go, and taken back with it.
set -euo pipefail

die() { echo "vfio-gpu: $*" >&2; exit 1; }
devices=${STELA_VFIO_SYSFS:-/sys/bus/pci/devices}
# Discrete RDNA 4 boards (crates/stela-driver-gpu/src/amdgpu_chip.rs).
amd_cards=(0x7550 0x7590)

gpu() {
  if [[ -n ${STELA_VFIO_GPU:-} ]]; then
    echo "$STELA_VFIO_GPU"
    return
  fi
  local device
  for device in "$devices"/*; do
    [[ $(<"$device/vendor") == 0x10de && $(<"$device/class") == 0x03* ]] || continue
    basename "$device"
    return
  done
  for device in "$devices"/*; do
    [[ $(<"$device/vendor") == 0x1002 && $(<"$device/class") == 0x03* ]] || continue
    [[ " ${amd_cards[*]} " == *" $(<"$device/device") "* ]] || continue
    basename "$device"
    return
  done
  die "no NVIDIA or RDNA 4 Radeon display function"
}

# The functions of the card's slot, which go to the guest together.
functions() {
  local device
  for device in "$devices/${1%.*}".*; do basename "$device"; done
}

driver() {
  local link="$devices/$1/driver"
  if [[ -e $link ]]; then basename "$(readlink -f "$link")"; else echo none; fi
}

status() {
  local card function device
  card=$(gpu)
  echo "card $card: $(lspci -s "$card" | cut -d' ' -f2-)"
  echo "boot VGA: $(<"$devices/$card/boot_vga")"
  for function in $(functions "$card"); do
    echo "$function: driver $(driver "$function"), IOMMU group $(basename "$(readlink "$devices/$function/iommu_group")")"
  done
  for device in "$devices/$card/iommu_group/devices"/*; do
    [[ $(basename "$device") == "${card%.*}".* ]] && continue
    echo "WARNING: $(basename "$device") shares the card's IOMMU group"
  done
  local displays=()
  for device in "$devices"/*; do
    [[ $(<"$device/class") == 0x03* && $(basename "$device") != "${card%.*}".* ]] || continue
    displays+=("$(basename "$device")")
  done
  if ((${#displays[@]})); then
    echo "other display functions: ${displays[*]}: this desktop can stay up"
  else
    echo "no other display function: a run needs --stop-desktop and ends this desktop"
  fi
}

bind() {
  local function=$1
  local current
  current=$(driver "$function")
  if [[ $current != none ]]; then
    echo "$function" >"$devices/$function/driver/unbind"
  fi
  echo vfio-pci >"$devices/$function/driver_override"
  echo "$function" >/sys/bus/pci/drivers_probe
  [[ $(driver "$function") == vfio-pci ]] || die "$function did not bind to vfio-pci"
  # An FLR of a card whose GSP has served work never completes and takes the
  # link with it. Without it QEMU resets the slot, which does bring it back.
  if [[ -e $devices/$function/reset_method ]]; then
    echo >"$devices/$function/reset_method"
  fi
}

release() {
  local function=$1
  if [[ $(driver "$function") == vfio-pci ]]; then
    echo "$function" >"$devices/$function/driver/unbind"
  fi
  echo >"$devices/$function/driver_override"
}

# Release the framebuffer consoles, which hold the card's driver.
release_consoles() {
  local console
  for console in /sys/class/vtconsole/vtcon*; do
    if grep -q 'frame buffer' "$console/name"; then
      echo 0 >"$console/bind"
    fi
  done
}

# Stop what holds an NVIDIA card: its services, consoles and modules.
unload_nvidia() {
  systemctl stop nvidia-persistenced.service 2>/dev/null || true
  release_consoles
  local module
  for module in nvidia_uvm nvidia_drm nvidia_modeset nvidia; do
    if [[ -d /sys/module/$module ]]; then
      modprobe -r "$module" || die "cannot unload $module: something still holds the card"
    fi
  done
}

restore() {
  local function console
  set +e
  for function in "${members[@]}"; do release "$function"; done
  for group in "${groups[@]}"; do
    [[ -e /dev/vfio/$group ]] && chown root:root "/dev/vfio/$group"
  done
  if ((nvidia)); then
    modprobe nvidia_drm
    modprobe nvidia_uvm
    systemctl start nvidia-persistenced.service 2>/dev/null
  fi
  for function in "${members[@]}"; do
    echo "$function" >/sys/bus/pci/drivers_probe
  done
  for console in /sys/class/vtconsole/vtcon*; do
    echo 1 >"$console/bind" 2>/dev/null
  done
  if ((stopped)); then
    if ((autologin)); then
      printf '[Seat:*]\nautologin-user=%s\nautologin-user-timeout=0\n' "$owner" >"$autologin_conf"
    fi
    systemctl start display-manager.service
    if ((autologin)); then
      # LightDM reads its configuration as it starts; later starts ask again.
      sleep 15
      rm -f "$autologin_conf"
    fi
  fi
  set -e
}

run() {
  local stop_desktop=0 seconds=3600
  autologin=0
  autologin_conf=/etc/lightdm/lightdm.conf.d/99-stela-vfio-autologin.conf
  while (($#)); do
    case $1 in
      --stop-desktop) stop_desktop=1 ;;
      --autologin) autologin=1 ;;
      --seconds) seconds=$2; shift ;;
      --) shift; break ;;
      *) die "unknown option $1" ;;
    esac
    shift
  done
  (($#)) || die "no command"
  ((EUID == 0)) || die "run needs root"
  owner=${SUDO_USER:-${STELA_VFIO_USER:-}}
  [[ -n $owner && $owner != root ]] || die "set STELA_VFIO_USER to the user who runs the command"

  local card
  card=$(gpu)
  mapfile -t members < <(functions "$card")
  # The functions of a slot may sit in IOMMU groups of their own (a
  # Radeon's HDMI audio does on some boards): QEMU opens each.
  groups=()
  local member
  for member in "${members[@]}"; do
    groups+=("$(basename "$(readlink "$devices/$member/iommu_group")")")
  done
  stopped=0
  nvidia=0
  trap restore EXIT
  modprobe vfio-pci
  if ((stop_desktop)); then
    systemctl stop display-manager.service
    stopped=1
    sleep 3
  fi
  case $(driver "$card") in
    nvidia) nvidia=1; unload_nvidia ;;
    # amdgpu stays loaded for the host's other GPUs; only this card leaves it.
    # The card is bound to amdgpu when the run starts and goes back to it, as
    # Navi 48 passthrough reports recommend: an early vfio-pci binding meets
    # the Radeon reset bug.
    amdgpu) release_consoles ;;
  esac
  local function
  for function in "${members[@]}"; do bind "$function"; done
  for group in "${groups[@]}"; do chown "$owner" "/dev/vfio/$group"; done
  ulimit -l unlimited
  local code=0
  # Stopping the service ends the command too: first let the card write no
  # more to memory QEMU is about to unmap.
  trap 'for function in "${members[@]}"; do setpci -s "$function" COMMAND=0:4; done; kill "$child" 2>/dev/null' TERM
  # setpriv, not runuser: runuser's PAM session (pam_limits) would lower the
  # memory-lock limit back to the user's, below what VFIO pins for guest RAM.
  local home
  home=$(getent passwd "$owner" | cut -d: -f6)
  timeout --kill-after=30 "$seconds" env HOME="$home" USER="$owner" LOGNAME="$owner" \
    setpriv --reuid="$owner" --regid="$(id -g "$owner")" --init-groups -- "$@" &
  child=$!
  wait "$child" || code=$?
  wait "$child" 2>/dev/null || true
  return "$code"
}

case ${1:-} in
  status) status ;;
  run) shift; run "$@" ;;
  *) die "usage: vfio-gpu.sh status | run [--stop-desktop] [--seconds N] -- COMMAND..." ;;
esac
