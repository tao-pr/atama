#!/usr/bin/env bash
# Writes each JSON object on stdin to its own file,
# <dir>/<prefix><value>.json, where <value> is the string at <key> in the
# object. The prefix is empty by default. The objects can be one
# per line or pretty JSON, one after the other. Prints the path of each
# file, one per line, in the order of the input.
#
# Fails if a value is not a JSON object, or its <key> is not a string of
# letters, digits, `_` and `-`, so that a value cannot name a file in
# another directory.
#
# Fails if the prefix has a character that is not a letter, a digit, `_`
# or `-`.
#
# Usage: write_json_by_key.sh <dir> <key> [prefix]
set -euo pipefail

usage="usage: write_json_by_key.sh <dir> <key> [prefix]"
dir="${1:?$usage}"
key="${2:?$usage}"
prefix="${3:-}"
if [[ ! "$prefix" =~ ^[A-Za-z0-9_-]*$ ]]; then
  echo "write_json_by_key.sh: \`$prefix\` is not a file name prefix" >&2
  exit 1
fi
# One compact JSON value per line, whatever the input format is.
values="$(jq -c .)"
mkdir -p "$dir"
while IFS= read -r value; do
  [[ -z "$value" ]] && continue
  name="$(jq -r --arg key "$key" 'if type == "object" then .[$key] // "" else "" end | strings' <<<"$value")"
  if [[ ! "$name" =~ ^[A-Za-z0-9_-]+$ ]]; then
    echo "write_json_by_key.sh: \`$key\` is not a file name in $value" >&2
    exit 1
  fi
  file="$dir/$prefix$name.json"
  jq . <<<"$value" >"$file"
  printf '%s\n' "$file"
done <<<"$values"
