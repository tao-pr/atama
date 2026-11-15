#!/usr/bin/env bash
# Prints one JSON object on each line for each model in <dir>/gguf.jsonl
# that has a GGUF model for Ollama. The object has the provider, the name,
# the params, the release date, the short description and the URL of the
# model from <dir>/models-<provider>.json, and the GGUF models:
#
#   {"provider", "name", "params", "release_date", "short_description",
#    "url", "gguf_models": [{"ollama_name", "url"}]}
#
# A value that the models-*.json files do not give is null.
#
# Usage: join_gguf_models.sh <dir>
set -euo pipefail
shopt -s nullglob

dir="${1:?usage: join_gguf_models.sh <dir>}"
files=("$dir"/models-*.json)
models='[]'
if ((${#files[@]})); then
  models="$(jq -cs . "${files[@]}")"
fi
jq -c --argjson m "$models" '
  ($m | map({key: .provider, value: .models}) | from_entries) as $by
  | select((.gguf_models // []) | length > 0)
  | . as $g
  | (($by[$g.provider] // []) | map(select(.name == $g.model)) | first) as $info
  | {
      provider: $g.provider,
      name: $g.model,
      params: $info.params,
      release_date: $info.release_date,
      short_description: $info.short_description,
      url: $info.url,
      gguf_models: $g.gguf_models
    }
' "$dir/gguf.jsonl"
