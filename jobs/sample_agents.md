# Research and report

Finds the latest stable Rust release on the web, saves notes about it, and
lets a coding agent write and run a small report script. This file is a sample
that shows two ways to configure an `agent:<type>` action. Every file goes
into `./output/agents/`.

Prerequisites:

- A chat model that supports tool calls, on a server with an OpenAI compatible
  `/chat/completions` endpoint. The sample uses Ollama on `localhost:11434`:

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

- The `Write report script` task reads its model from the environment. Set
  these variables before you run the job:

  ```bash
  export ATAMA_AGENT_BASE_URL=http://localhost:11434/v1
  export ATAMA_AGENT_MODEL=qwen3.5:0.8b
  # Only if the server needs a key, for example https://api.openai.com/v1
  export ATAMA_AGENT_API_KEY=<your key>
  ```

The `Write report script` task lets the model run bash commands in the
directory where you start Atama. Read its task before you run the job.

To run the job:

```bash
cargo run -- jobs/sample_agents.md
```

To see what each agent sends to the model, what the model replies, and which
tools it calls with which arguments, add `--debug`. The debug output goes to
stderr. This command saves it in `debug.log`, and the task log stays in the
terminal:

```bash
cargo run -- --debug jobs/sample_agents.md 2> debug.log
```

If an agent fails or gives a strange answer, read `debug.log`. For example, a
small model can call a tool with wrong arguments, or stop before it uses a
tool.

## Prepare

Make the output directory.

- actions:
  - bash mkdir -p ./output/agents

## Research

A `websearch` agent with every option in the action itself: `model=` and
`base_url=` select the model, and `max_steps=6` allows at most 6 model
replies. The other words are the task. The agent can call `fetch_url` to read
web pages. Its final reply is the output of the task.

- actions:
  - agent:websearch model=qwen3.5:0.8b base_url=http://localhost:11434/v1 max_steps=6 Find the latest stable Rust release on https://blog.rust-lang.org and list its version, its release date and three notable changes as a short markdown list.
- if_fail: retry=1

## Save notes

Write the notes from Research to a file. `tee` also passes them on as the
output of the task.

- actions:
  - bash tee ./output/agents/rust-release.md

## Write report script

A `coder` agent without options in the action. It gets its model, server and
API key from the `ATAMA_AGENT_*` environment variables, and the default of 10
steps. The agent can call `bash`. The notes from Save notes are its input.

- actions:
  - agent:coder Write a bash script ./output/agents/report.sh that prints the name and the line count of each .md file in ./output/agents. Make it executable and run it once. Change no files outside ./output/agents. Reply with the output of the script.
  - bash tee ./output/agents/report.txt

## List output

Print all the files that the job wrote.

- actions:
  - bash ls -a ./output/agents

## Research failed

Runs only if Research fails, for example if the model server is not running.
`mock` stands in for an alert that Atama cannot send yet.

- begin_if: task(Research) fail
- actions:
  - mock send an alert
