#!/usr/bin/env bash
set -euo pipefail

# Keep only the newest audio artifacts in operational/test cache directories.
# This script deliberately does not scan model asset directories.

MAX_FILES="${AUDIO_CACHE_MAX_FILES:-100}"

if ! [[ "$MAX_FILES" =~ ^[0-9]+$ ]] || [ "$MAX_FILES" -lt 1 ]; then
  echo "AUDIO_CACHE_MAX_FILES must be a positive integer" >&2
  exit 2
fi

if [ "$#" -gt 0 ]; then
  SEARCH_DIRS=("$@")
else
  SEARCH_DIRS=(
    "/home/t2_enroll_ai/rust_enrollment/tmp"
    "/home/t2_enroll_ai/model-service-logs"
  )
fi

tmp_file="$(mktemp)"
trap 'rm -f "$tmp_file"' EXIT

for dir in "${SEARCH_DIRS[@]}"; do
  if [ -d "$dir" ]; then
    find "$dir" -type f \
      \( -iname '*.wav' -o -iname '*.mp3' -o -iname '*.pcm' -o -iname '*.ogg' -o -iname '*.webm' -o -iname '*.flac' \) \
      -printf '%T@ %p\n'
  fi
done | sort -nr > "$tmp_file"

total="$(wc -l < "$tmp_file" | tr -d ' ')"
if [ "$total" -le "$MAX_FILES" ]; then
  echo "audio cache prune: kept $total file(s), limit $MAX_FILES"
  exit 0
fi

delete_count=$((total - MAX_FILES))
tail -n "$delete_count" "$tmp_file" | while IFS= read -r line; do
  path="${line#* }"
  if [ -n "$path" ] && [ -f "$path" ]; then
    rm -f -- "$path"
    echo "deleted $path"
  fi
done

echo "audio cache prune: deleted $delete_count old file(s), kept $MAX_FILES newest"
