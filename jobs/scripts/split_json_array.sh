#!/usr/bin/env bash
# Writes each element of the JSON array on stdin to its own file,
# <dir>/result-<n>.json, with n from 1. Prints the path of each file, one
# per line, in the order of the array.
#
# Usage: split_json_array.sh <dir>
set -euo pipefail

dir="${1:?usage: split_json_array.sh <dir>}"
input="$(cat)"
if ! jq -e 'type == "array"' <<<"$input" >/dev/null; then
  echo "split_json_array.sh: the input is not a JSON array" >&2
  exit 1
fi
mkdir -p "$dir"
count="$(jq length <<<"$input")"
for ((i = 0; i < count; i++)); do
  file="$dir/result-$((i + 1)).json"
  jq ".[$i]" <<<"$input" >"$file"
  printf '%s\n' "$file"
done
