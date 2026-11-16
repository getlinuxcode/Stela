#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Runs Claude Code on this repository in a tmux session that outlives the
# graphical desktop, so the work goes on while the Radeon is lent to a VM.
#
# With one GPU, lending it (vfio-gpu.sh run --stop-desktop) stops the X
# server, and with it every graphical program, the Claude desktop app
# included. A tmux session started from a terminal is not one of them:
# logind keeps a user's processes when their session ends
# (KillUserProcesses=no, the Debian and Mint default), so the agent keeps
# working through the run, and the desktop comes back when it ends. The
# user's systemd manager, which tmux's scope belongs to, must linger, or it
# stops with the last session (this script turns lingering on).
#
#   xtask/scripts/amd-agent.sh            start the session, or attach to it
#   xtask/scripts/amd-agent.sh --resume   the same, resuming the last conversation
#
# Follow it from another machine with `ssh -t USER@HOST tmux attach -t stela`
# (openssh-server on this one), or from Claude's Remote Control.
set -euo pipefail

session=stela
repo=$(cd "$(dirname "$0")/../.." && pwd)

command -v tmux >/dev/null || { echo "amd-agent: install tmux (sudo apt install tmux)" >&2; exit 1; }
if [[ $(loginctl show-session "${XDG_SESSION_ID:-self}" -p Remote --value 2>/dev/null) != yes ]] &&
   grep -Eq '^\s*KillUserProcesses\s*=\s*yes' /etc/systemd/logind.conf 2>/dev/null; then
  echo "amd-agent: logind kills a session's processes when it ends; the agent would not survive" >&2
  exit 1
fi

# Without lingering, systemd stops the user's manager when their last
# session ends, and tmux's scope with it: keep it running (no root needed).
if [[ $(loginctl show-user "$USER" -p Linger --value 2>/dev/null) != yes ]]; then
  loginctl enable-linger "$USER" || { echo "amd-agent: run loginctl enable-linger $USER" >&2; exit 1; }
fi

# The CLI on PATH, else the one the desktop app ships (newest first).
claude=$(command -v claude || true)
if [[ -z $claude ]]; then
  claude=$(ls -t "$HOME"/.config/Claude/claude-code/*/*/claude 2>/dev/null | head -1 || true)
fi
[[ -n $claude ]] || { echo "amd-agent: no Claude Code CLI found" >&2; exit 1; }

if ! tmux has-session -t "$session" 2>/dev/null; then
  args=()
  [[ ${1:-} == --resume ]] && args+=(--continue)
  tmux new-session -d -s "$session" -c "$repo" "$claude" "${args[@]}"
fi
exec tmux attach -t "$session"
