#!/usr/bin/env bash
# Runs a command against a virtual X display, as CI runs the approved
# scenarios: Xvfb, and the game's window focused once it appears, which a
# desktop's window manager would do. Without focus the game ignores bound
# keys (Use, movement) the way it does when alt-tabbed out.
#
#   scripts/headless-display.sh cargo run --profile play -p approved_tests -- crafting_table
#
# Needs Xvfb and xdotool; on a machine without a GPU, Mesa's lavapipe
# (VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json) renders.
set -euo pipefail

# A display of its own: a shared one dies with whichever run started it.
# No abstract socket either (`-nolisten local`): those live in the network
# namespace, which host-network containers share, so a client could reach
# another container's server of the same number.
number=99
while [[ -e /tmp/.X11-unix/X$number || -e /tmp/.X$number-lock ]]; do
  number=$((number + 1))
done
export DISPLAY=:$number
Xvfb "$DISPLAY" -screen 0 1920x1080x24 -nolisten tcp -nolisten local >/dev/null 2>&1 &
xvfb=$!
trap 'kill "$xvfb" 2>/dev/null || true' EXIT
for _ in $(seq 50); do
  xdpyinfo -display "$DISPLAY" >/dev/null 2>&1 && break
  sleep 0.2
done

# Focus each new top-level window as it maps.
(
  # Not the script's exit trap, which stops Xvfb, nor its errexit.
  trap - EXIT
  set +e
  focused=""
  while true; do
    window=$(xdotool search --onlyvisible --name . 2>/dev/null | tail -1 || true)
    if [[ -n $window && $window != "$focused" ]]; then
      xdotool windowfocus "$window" 2>/dev/null && focused=$window
    fi
    sleep 0.5
  done
) &
focus=$!
trap 'kill "$focus" "$xvfb" 2>/dev/null || true' EXIT

"$@"
