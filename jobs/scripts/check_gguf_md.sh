#!/usr/bin/env bash
# Makes sure that the Markdown file <md> names each model and each Ollama
# GGUF name in <jsonl>, the output of join_gguf_models.sh. Prints each name
# that is missing, and exits with 1 if a name is missing or <md> is empty.
#
# Usage: check_gguf_md.sh <jsonl> <md>
set -euo pipefail

usage="usage: check_gguf_md.sh <jsonl> <md>"
jsonl="${1:?$usage}"
md="${2:?$usage}"
if [[ ! -s "$md" ]]; then
  echo "$md is missing or empty"
  exit 1
fi
missing=0
while IFS= read -r name; do
  if ! grep -qF -- "$name" "$md"; then
    echo "missing in $md: $name"
    missing=1
  fi
done < <(jq -r '.name, .gguf_models[].ollama_name' "$jsonl")
exit "$missing"
