```config
local-spark2: model=hf.co/XHToken/Spark-X2.6-1.7B-GGUF:Q4_K_M, base_url=http://localhost:11434/v1
local-qwen2: model=qwen2.5-coder:3b, base_url=http://localhost:11434/v1, token_budget=60000
```

# Code and explain

Reads a coding task from a markdown file, and lets two `coder` agents, each
with its own model, write a short Python program for it.
Then a `summarizer` agent explains each program. This file is a sample of a
`coder` agent, its `bash` tool and `actions-parallel`. Every file goes into
`./output/coder/`.

Prerequisites:

- Ollama on `localhost:11434`, with the models of the two profiles. To set
  up Ollama, see `jobs/sample_agents.md`. To download the models, run
  `ollama pull hf.co/XHToken/Spark-X2.6-1.7B-GGUF:Q4_K_M` and
  `ollama pull qwen2.5-coder:3b`.
- `python3`, to run the program that the agent writes.

The `local-qwen2` profile has `token_budget=60000`. Each agent run that
uses this profile can use at most 60,000 tokens.

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

Two `coder` agents write the code. `actions-parallel` gives each one the
task from Read task as its input. `max=1` runs one agent at a time, so that
two local models do not share the CPU. Each agent writes a Python program
of at most 10 lines to its own file with its `bash` tool, and runs it to
test it. `thinking=false` makes the small models faster on a CPU. The task
passes no output on.

Each request sends the whole conversation again, so the tokens grow with
each step. A token budget stops an agent that loops too long. The
`local-spark2` agent sets `token_budget=40000` on its action. The
`local-qwen2` agent has no `token_budget=`, so it uses the budget of its
profile. If an agent uses its budget before its final reply, it fails.
Then `if_fail: retry=0` fails the task without a retry.

- actions-parallel:max=1
  - agent:coder model=local-spark2 thinking=false max_steps=8 token_budget=40000 task="Write a Python program of at most 10 lines that does the coding task below. Save it to ./output/coder/solution-spark2.py with the bash tool, then run it with python3 to test it. If it fails, fix it."
  - agent:coder model=local-qwen2 thinking=true max_steps=8 task="Write a Python program of at most 10 lines that does the coding task below. Save it to ./output/coder/solution-qwen2.py with the bash tool, then run it with python3 to test it. If it fails, fix it."
- if_fail: retry=0

## Run code

Run both programs at the same time, so that the log shows their output. A
model wrote each program, so `bash-sandbox` runs it in the macOS sandbox,
without network and with files only in the working directory. The task
fails if an agent wrote no program, or a program fails.

- actions-parallel:
  - bash-sandbox python3 ./output/coder/solution-spark2.py
  - bash-sandbox python3 ./output/coder/solution-qwen2.py

## Explain spark2 code

Explain the program of the `local-spark2` agent with spark2.

- actions:
  - read_file ./output/coder/solution-spark2.py
  - agent:summarizer model=local-spark2 thinking=false expertise="Python programming" max_output="5 sentences" task="Explain the logic of this Python program in at most 5 lines, one sentence on each line."
  - write_file ./output/coder/explanation-spark2.txt

## Explain qwen2 code

Explain the program of the `local-qwen2` agent with spark2.

- begin_if: task(Run code) succeed
- actions:
  - read_file ./output/coder/solution-qwen2.py
  - agent:summarizer model=local-spark2 thinking=false expertise="Python programming" max_output="5 sentences" task="Explain the logic of this Python program in at most 5 lines, one sentence on each line."
  - write_file ./output/coder/explanation-qwen2.txt
