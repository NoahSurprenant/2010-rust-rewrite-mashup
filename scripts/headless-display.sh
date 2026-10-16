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

display=${HEADLESS_DISPLAY:-:99}
export DISPLAY=$display
if ! xdpyinfo -display "$display" >/dev/null 2>&1; then
  Xvfb "$display" -screen 0 1920x1080x24 -nolisten tcp >/dev/null 2>&1 &
  xvfb=$!
  trap 'kill "$xvfb" 2>/dev/null || true' EXIT
  for _ in $(seq 50); do
    xdpyinfo -display "$display" >/dev/null 2>&1 && break
    sleep 0.2
  done
fi

# Focus each new top-level window as it maps.
(
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
trap 'kill "$focus" 2>/dev/null || true; kill "${xvfb:-}" 2>/dev/null || true' EXIT

"$@"
