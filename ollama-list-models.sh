#!/usr/bin/env bash
# List the Ollama models as a table.
# Usage: ollama-list-models.sh [host]
#   host  The Ollama host, for example 192.168.1.10, gpu-box:11434
#         or https://ollama.example.com. Default: localhost.
#         A host without a scheme gets http:// and, if it has
#         no port, the port 11434. A full URL stays as it is.
set -euo pipefail

host="${1:-localhost}"

if [[ "$host" != *://* ]]; then
  if [[ "$host" != *:* ]]; then
    host="$host:11434"
  fi
  host="http://$host"
fi
host="${host%/}"

curl -s "$host/api/tags" | jq -r '
  ["MODEL","SIZE_GB","FORMAT","FAMILY","PARAMS","QUANT","CONTEXT","CAPABILITIES"],
  (.models[] | [
    .model,
    (.size / 1e9 * 100 | round / 100),
    .details.format,
    .details.family,
    .details.parameter_size,
    .details.quantization_level,
    .details.context_length,
    (.capabilities // [] | join(","))
  ])
  | @tsv' | column -t -s $'\t'
