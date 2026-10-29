```config
local-qwen: model=qwen3.5:0.8b, base_url=http://localhost:11434/v1
openai-gpt5: model=gpt-5, base_url=https://api.openai.com/v1, api_key=OPENAI_API_KEY
```

# Research and summarize

Searches the web for the most common agent loop patterns for agentic
engineering, saves the raw search results, and lets a `summarizer` agent
analyse them. The patterns must be model-agnostic, so that they work with
any model, and they must fit most common AI workflows. This file is a sample
of a `websearch` action and a `summarizer` agent. Every file goes into
`./output/agents/`.

The ```` ```config ```` block at the start of the file defines two model
profiles. Each line has a profile name, the model identifier, the base URL
of the server, and, if the server needs a key, the name of the environment
variable that holds the key. An agent action selects a profile with
`model=<profile>`. The job file never holds the key itself.

- `local-qwen` is `qwen3.5:0.8b` on the local Ollama server. Ollama needs
  no key.
- `openai-gpt5` is `gpt-5` on OpenAI. Atama reads its key from
  `OPENAI_API_KEY`. The sample does not use it. To use it, change
  `model=local-qwen` to `model=openai-gpt5` and set `OPENAI_API_KEY`.

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

- The `Summarize research` task uses the `local-qwen` profile, so it needs
  no API key. If you change it to `openai-gpt5`, set the key before you run
  the job:

  ```bash
  export OPENAI_API_KEY=<your key>
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

Delete the files of an earlier run, and make the output directory again.
`rm -rf` deletes the whole directory, because Atama gives `*` to a command
as plain text, without a shell, so `rm -f ./output/agents/*` deletes
nothing.

- actions:
  - bash rm -rf ./output/agents
  - bash mkdir -p ./output/agents

## Research

A `websearch` action searches `topic=` with Tavily and gets the top 5
results (`max_results=3`). `fetch_content=true` also gets the page of each
result as markdown. The quotes let a value have spaces. The action asks no
model. Its output is the JSON response body from Tavily, without changes.

- actions:
  - websearch max_results=3 fetch_content=false topic="most common model-agnostic agent loop patterns for agentic engineering and AI workflows"
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

Write each search result to its own JSON file: the first result goes to
`./output/agents/research-raw/result-1.json`, the second to `result-2.json`,
and so on. `jobs/scripts/split_json_array.sh` does the split with `jq`,
because a `bash` action passes its words to the command without a shell.
The script prints the path of each file, one per line, so the list of files
is the output of the task. If the input is not a JSON array, the script
fails.

- actions:
  - bash ./jobs/scripts/split_json_array.sh ./output/agents/research-raw

## Summarize research

Summarize each search result on its own, with a `summarizer` agent that
uses the `local-qwen` profile: `qwen3.5:0.8b` on the local Ollama server.

The input of the task is the list of files from Save raw research. `xargs`
gives the paths to `jq -c .`, which writes the JSON of each file on one
line. `foreach` then runs the agent once for each line, so each run gets
one search result. Each run writes the summary of its result in markdown.
The output of the task is the summaries, one after the other.

- actions:
  - bash xargs jq -c .
  - foreach agent:summarizer model=local-qwen expertise="agentic AI engineering" max_output="40%" task="You are given one JSON object, which you will convert into markdown. The object has a title, a URL, and the page content, which you will convert into a heading (from title) and a summary of the content (from 'content' field). The summary text should still keep the URL on the top of the summary, then the summarized text below it. The summary should be concise, clear, and informative."

## Save summary

Write the summary to a file. `tee` also passes it on as the output of the
task.

- actions:
  - bash tee ./output/agents/research-summary.md

## Print summary

Print the summary file that the job wrote.

- actions:
  - bash cat ./output/agents/research-summary.md
