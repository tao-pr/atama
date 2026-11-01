```config
local-spark2: model=hf.co/XHToken/Spark-X2.6-1.7B-GGUF:Q4_K_M, base_url=http://localhost:11434/v1
```

# Code and explain

Reads a coding task from a markdown file, lets a `coder` agent write a short
Python program for it, and lets a `summarizer` agent explain the code. This
file is a sample of a `coder` agent and its `bash` tool. Every file goes into
`./output/coder/`.

Prerequisites:

- Ollama on `localhost:11434`, with the model of the `local-spark2` profile.
  To set up Ollama, see `jobs/sample_agents.md`. To download the model, run
  `ollama pull hf.co/XHToken/Spark-X2.6-1.7B-GGUF:Q4_K_M`.
- `python3`, to run the program that the agent writes.

The `coder` agent can run any command with your permissions. Run the job only
in a directory that you trust.

To run the job:

```bash
cargo run -- jobs/sample_coder.md
```

## Prepare

Delete the files of an earlier run, and make the output directory again.

- actions:
  - bash rm -rf ./output/coder
  - bash mkdir -p ./output/coder

## Read task

Read the coding task. The output is the markdown of the task, which is the
input of the next task.

- actions:
  - read_file ./jobs/tasks/coding_task.md

## Write code

A `coder` agent reads the task and writes a Python program of at most 10
lines to `./output/coder/solution.py` with its `bash` tool. It also runs
the program to test it. `thinking=false` makes the small model faster on a
CPU.

- actions:
  - agent:coder model=local-spark2 thinking=false max_steps=8 task="Write a Python program of at most 10 lines that does the coding task below. Save it to ./output/coder/solution.py with the bash tool, then run it with python3 to test it. If it fails, fix it."
- if_fail: retry=1

## Run code

Run the program, so that the log shows its output. A model wrote the
program, so `bash-sandbox` runs it in the macOS sandbox, without network
and with files only in the working directory. The task fails if the agent
wrote no program, or the program fails.

- actions:
  - bash-sandbox python3 ./output/coder/solution.py

## Explain code

Read the program, and let a `summarizer` agent explain its logic in at most
5 lines. `write_file` saves the explanation and passes it on, so the log
shows it.

- actions:
  - read_file ./output/coder/solution.py
  - agent:summarizer model=local-spark2 thinking=false expertise="Python programming" max_output="5 sentences" task="Explain the logic of this Python program in at most 5 lines, one sentence on each line."
  - write_file ./output/coder/explanation.txt
