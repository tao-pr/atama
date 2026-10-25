# Research and summarize

Searches the web for the most common agent loop patterns for agentic
engineering, saves the raw search results, and lets a `summarizer` agent
analyse them. The patterns must be model-agnostic, so that they work with
any model, and they must fit most common AI workflows. This file is a sample
of a `websearch` action and a `summarizer` agent. Every file goes into
`./output/agents/`.

Prerequisites:

- A chat model on a server with an OpenAI compatible `/chat/completions`
  endpoint, for the `Summarize research` task. The sample uses Ollama on
  `localhost:11434`:

  If you don't have Ollama installed, see https://ollama.com/download.

  1. Start the Ollama server in a separate terminal. The command keeps
     running. If the Ollama desktop app is open, the server already runs,
     and you can skip this step.

     ```bash
     ollama serve
     ```

  2. Download the model:

     ```bash
     ollama pull qwen3.5:0.8b
     ```

  3. Load the model into memory and keep it there until the server stops.
     `ollama serve` serves every model that you pulled, and it loads a model
     only at its first request. This request loads `qwen3.5:0.8b` now, so the
     first agent step does not wait for it:

     ```bash
     curl http://localhost:11434/api/generate -d '{"model": "qwen3.5:0.8b", "keep_alive": -1}'
     ```

     To make sure that the model is loaded, run `ollama ps`. To unload it,
     run `ollama stop qwen3.5:0.8b`.

- The `Research` task searches the web with Tavily, so it needs a Tavily
  API key:

  ```bash
  export TAVILY_API_KEY=<your Tavily key>
  ```

  The search uses Tavily API credits.

- The `Extract results` task uses `jq` to read the JSON. To install it, see
  https://jqlang.org/download/.

- The `Summarize research` task sets its model and server in the action:
  `qwen3.5:0.8b` on the local Ollama server. Atama reads the API key of the
  model server only from the `ATAMA_AGENT_API_KEY` environment variable,
  never from the job file. Ollama needs no key, so you can leave it unset.
  If you change the action to a server that needs a key, for example
  `https://api.openai.com/v1`, set the key before you run the job:

  ```bash
  export ATAMA_AGENT_API_KEY=<your key>
  ```

To run the job:

```bash
cargo run -- jobs/sample_agents.md
```

To see the Tavily response and what the summarizer sends to the model and
what the model replies, add `--debug`. The debug output goes to stderr. This command saves it in `debug.log`, and the task log stays in the
terminal:

```bash
cargo run -- --debug jobs/sample_agents.md 2> debug.log
```

If a task fails or gives a strange answer, read `debug.log`. For example,
the input can be too long for a small model.

## Prepare

Make the output directory.

- actions:
  - bash mkdir -p ./output/agents

## Research

A `websearch` action searches `topic=` with Tavily and gets the top 5
results (`max_results=5`). `fetch_content=true` also gets the page of each
result as markdown. The quotes let a value have spaces. The action asks no
model. Its output is the JSON response body from Tavily, without changes.

- actions:
  - websearch max_results=5 fetch_content=true topic="most common model-agnostic agent loop patterns for agentic engineering and AI workflows"
- if_fail: retry=1

## Extract results

Keep only the `results` array of the Tavily response, so that the next tasks
do not get the other fields, such as `query` and `response_time`. Atama gives
the output of Research to `jq` on stdin. `jq .results` writes the array as
one JSON value. `.results[]` would write each result as a separate JSON
value, and then the output is not one JSON document.

- actions:
  - bash jq .results

## Save raw research

Write the search results to a JSON file. `tee` also passes them on as the
output of the task.

- actions:
  - bash tee ./output/agents/research-raw.json

## Summarize research

A `summarizer` agent that uses the local Ollama model. `model=` and
`base_url=` select `qwen3.5:0.8b` on `localhost:11434`. The API key comes
from `ATAMA_AGENT_API_KEY`, if it is set.
`cat` reads the saved search results, and its output is the input of the
agent. `task=` tells the model what to take from the search results.
`expertise=` sets the field of the reader, and `max_output=` limits the
summary to 40% of the length of the input. The summary keeps the names,
numbers and source URLs that are important to the key points.

- actions:
  - bash cat ./output/agents/research-raw.json
  - agent:summarizer model=qwen3.5:0.8b base_url=http://localhost:11434/v1 expertise="agentic AI engineering" max_output="40%" task="You receive an input of a list of agentic loop patterns. You look at the content summarize into just the key points, remember to keep the names, source URLs. The output should be compiled into a single markdown file format with headings, subheadings and bullet points. Each topic should be a heading, and its keypoint summary should be a bullet point list under it."

## Save summary

Write the summary to a file. `tee` also passes it on as the output of the
task.

- actions:
  - bash tee ./output/agents/research-summary.md

## Print summary

Print the summary file that the job wrote.

- actions:
  - bash cat ./output/agents/research-summary.md
