#!/usr/bin/env bash
# Writes each element of the JSON array on stdin to its own file,
# <dir>/result-<n>.json, with n from 1. Then it prints the array again, so
# that the next action or task gets the same input.
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
  jq ".[$i]" <<<"$input" >"$dir/result-$((i + 1)).json"
done
printf '%s\n' "$input"
