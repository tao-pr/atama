```config
local-spark2: model=hf.co/XHToken/Spark-X2.6-1.7B-GGUF:Q4_K_M, base_url=http://localhost:11434/v1
```

```json:search_query
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "One web search query, at most 12 words."
    }
  },
  "required": ["query"],
  "additionalProperties": false
}
```

```json:models
{
  "type": "object",
  "properties": {
    "provider": {
      "type": "string",
      "enum": ["openai", "huggingface", "anthropic", "grok", "gemini"],
      "description": "The provider of the input."
    },
    "models": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "name": {
            "type": "string",
            "description": "The name of the model."
          },
          "params" : {
            "type": "integer",
            "description": "The number of parameters of the model."
          },
          "url": {
            "type": "string",
            "description": "The URL of the model card page."
          },
          "release_date": {
            "type": "string",
            "description": "The release date of the model, in YYYY-MM-DD format."
          },
          "capabilities": {
            "type": "array",
            "items": {
              "type": "string"
            }
          }
        },
        "required": ["name", "url", "capabilities"]
      }
    }
  },
  "required": ["provider", "models"],
  "additionalProperties": false
}
```

# Auto Model Research

Research the frontier models of OpenAI, Hugging Face, Anthropic, Grok and
Gemini, and small GGUF models for Ollama on a CPU. The research is about
tool calls, structured input and output, and text analysis. Needs Ollama
with `local-spark2`, `TAVILY_API_KEY` and `jq`.

```bash
cargo run -- jobs/auto_model_research.md
```

## Prepare

Make an empty output directory.

- actions:
  - bash rm -rf ./output/model_research
  - bash mkdir -p ./output/model_research

## Providers

One model provider on each line. The names are also file names.

- actions:
  - bash printf %s\n openai huggingface anthropic grok gemini
  - write_file ./output/model_research/providers.txt

## Generate Web Search Queries

`local-spark2` writes one query for each provider. The queries go to
`queries.txt`, one on each line, in the order of the providers.

- actions:
  - foreach agent:summarizer
    - model=local-spark2
    - thinking=false
    - output_schema=search_query
    - task=The input is the name of a model provider. Write one web search
      query that finds the latest frontier models of this provider, with the
      model name, model card URL, release date, number of parameters and
      capabilities, such as tool calls, structured output and text analysis.
  - bash jq -r .query
  - write_file ./output/model_research/queries.txt

## Running websearch

One Tavily search for each query. `jq` adds the provider of each result,
by its order, and keeps a short part. `search.jsonl` has one result on each
line.

- actions:
  - read_file ./output/model_research/queries.txt
  - foreach tavily_search max_results=5 include_answer=basic
  - bash jq -c -n --rawfile p ./output/model_research/providers.txt [($p|split("\n")),[inputs]]|transpose[]|select(.[1])|{provider:.[0],query:.[1].query,answer:.[1].answer,results:[.[1].results[]|{title,url,content}]}
  - write_file ./output/model_research/search.jsonl
- if_fail: retry=1

## Extract Models

`local-spark2` reads the models of each provider. `jq` sets the provider
again by its order, and each provider goes to `models-<provider>.json`.

- actions:
  - read_file ./output/model_research/search.jsonl
  - foreach agent:summarizer
    - model=local-spark2
    - thinking=false
    - output_schema=models
    - task=The input is the web search result of one model provider. List
      the frontier models in the result, with the name, model card URL,
      release date, number of parameters and capabilities of each model, such
      as tool calls, structured output and text analysis. Leave out a value
      that the result does not give.
  - bash jq -c -n --rawfile p ./output/model_research/providers.txt [($p|split("\n")),[inputs]]|transpose[]|select(.[1])|.[1]+{provider:.[0]}
  - bash ./jobs/scripts/write_json_by_key.sh ./output/model_research provider models-
