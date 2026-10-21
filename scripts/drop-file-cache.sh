#!/usr/bin/env bash
# Drops the page cache of every regular file under the paths given, so that
# files a CI job only wrote or unpacked (the play binaries, Minecraft's
# files restored from the Actions cache) are not charged to its memory
# while the games run. GNU dd's `nocache` flag with `count=0` advises the
# kernel to drop a whole file's cached pages (POSIX_FADV_DONTNEED); a sync
# first, as dirty pages are not dropped.
#
#   scripts/drop-file-cache.sh target/play iw4l-artifacts/minecraft-26.3
set -euo pipefail
sync
for path in "$@"; do
  [ -e "$path" ] || continue
  find "$path" -type f -print0 | while IFS= read -r -d '' file; do
    dd if="$file" iflag=nocache count=0 status=none 2>/dev/null || true
  done
done
