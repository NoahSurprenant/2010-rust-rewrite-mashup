#!/usr/bin/env bash
# The in-cluster scenario jobs (ci.yml's `scenario` matrix) a change needs.
#
#   scripts/ci-scenarios.sh all          every job
#   scripts/ci-scenarios.sh < paths      the jobs the changed paths (one a line) can affect
#   scripts/ci-scenarios.sh --self-test  checks the path rules below
#
# Prints the matrix as JSON ({"include":[...]}, empty when no job is needed).
# A path the rules do not name runs every job: shared engine, net, sim and
# render code can break any scenario. Only what the scenarios cannot reach
# runs none (docs, the Helm charts, packaging, the Windows updater and the
# workflows that only publish), and a scenario's own file runs its own jobs.
set -euo pipefail

# key|scenario|part|games|timeout|skate
JOBS=(
  'crafting|crafting_table||MW2 + Minecraft|60|false'
  'mm-world|minecraft_multiplayer|world|listen host + client|90|false'
  'mm-mobs|minecraft_multiplayer|mobs|listen host + client|90|false'
  'ds-visits|dedicated_server|visits|dedicated server + client|90|false'
  'ds-mobs|dedicated_server|mobs|dedicated server + client|90|false'
  'skate-listen|skate_multiplayer|listen|listen host + client|90|true'
  'skate-dedicated|skate_multiplayer|dedicated|dedicated server + client|90|true'
)

# The jobs one changed path needs: `all`, `none` or job keys.
jobs_for() {
  local scenarios=crates/approved_tests/src/scenarios
  case "$1" in
    docs/* | *.md | LICENSE | NOTICE | charts/* | packaging/* | crates/updater/* \
      | scripts/windows-update-handoff.ps1 | scripts/bench-marks.py | clippy.toml | rustfmt.toml \
      | .github/workflows/release.yml | .github/workflows/helm-charts.yml \
      | .github/workflows/dedicated-image.yml | .github/workflows/master-image.yml)
      echo none ;;
    "$scenarios/crafting_table.rs") echo crafting ;;
    "$scenarios/minecraft_multiplayer.rs") echo mm-world mm-mobs ;;
    "$scenarios/dedicated_server.rs") echo ds-visits ds-mobs ;;
    "$scenarios/skate_multiplayer.rs") echo skate-listen skate-dedicated ;;
    # The mob steps' callers: crafting_table (its arena), and the mobs parts.
    "$scenarios/mob_steps.rs") echo crafting mm-mobs ds-mobs ;;
    *) echo all ;;
  esac
}

# The job keys the paths on stdin need, one a line, or `all`.
select_keys() {
  local path key
  declare -A want=()
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    for key in $(jobs_for "$path"); do
      case $key in
        all) echo all; return ;;
        none) ;;
        *) want[$key]=1 ;;
      esac
    done
  done
  for key in "${!want[@]}"; do echo "$key"; done
}

matrix() {
  local keys=" $* " row key scenario part games timeout skate sep=''
  printf '{"include":['
  for row in "${JOBS[@]}"; do
    IFS='|' read -r key scenario part games timeout skate <<< "$row"
    case $keys in *" all "* | *" $key "*) ;; *) continue ;; esac
    printf '%s{"scenario":"%s","part":"%s","games":"%s","timeout":%s,"skate":%s}' \
      "$sep" "$scenario" "$part" "$games" "$timeout" "$skate"
    sep=,
  done
  printf ']}\n'
}

self_test() {
  local fails=0
  check() {
    local got
    got=$(printf '%s\n' "${@:2}" | select_keys | sort | tr '\n' ' ')
    if [ "$got" != "$1" ]; then echo "FAIL: ${*:2} -> '$got', want '$1'"; fails=$((fails + 1)); fi
  }
  check '' docs/RUN.md README.md charts/iw4l-dedicated/values.yaml crates/updater/src/lib.rs
  check '' .github/workflows/release.yml scripts/windows-update-handoff.ps1 packaging/windows/x.iss
  check 'all ' crates/net/src/lib.rs
  check 'all ' docs/RUN.md crates/sim/src/lib.rs
  check 'all ' .github/workflows/ci.yml
  check 'all ' scripts/ci-scenarios.sh
  check 'all ' crates/approved_tests/src/runner.rs
  check 'crafting ' crates/approved_tests/src/scenarios/crafting_table.rs
  check 'mm-mobs mm-world ' crates/approved_tests/src/scenarios/minecraft_multiplayer.rs docs/RUN.md
  check 'crafting ds-mobs mm-mobs ' crates/approved_tests/src/scenarios/mob_steps.rs
  check 'skate-dedicated skate-listen ' crates/approved_tests/src/scenarios/skate_multiplayer.rs
  check '' ''
  [ "$(matrix all | grep -o '"scenario"' | wc -l)" = 7 ] || { echo "FAIL: all is not 7 jobs"; fails=$((fails + 1)); }
  [ "$(matrix)" = '{"include":[]}' ] || { echo "FAIL: no keys is not an empty matrix"; fails=$((fails + 1)); }
  [ "$(matrix ds-mobs)" = '{"include":[{"scenario":"dedicated_server","part":"mobs","games":"dedicated server + client","timeout":90,"skate":false}]}' ] \
    || { echo "FAIL: ds-mobs matrix: $(matrix ds-mobs)"; fails=$((fails + 1)); }
  [ $fails = 0 ] && echo "ci-scenarios self-test ok"
  return $fails
}

case "${1:-}" in
  --self-test) self_test ;;
  all) matrix all ;;
  '') matrix $(select_keys) ;;
  *) echo "usage: $0 [all | --self-test] (changed paths on stdin)" >&2; exit 2 ;;
esac
