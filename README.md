# Atama

Atama is an automation pipeline as code. You describe a pipeline in a simple
markdown file, and Atama runs its tasks, with or without an agent of your choice.

## Pipeline format

A pipeline is a markdown file. The top-level heading names the pipeline, and each
second-level heading defines a task:

```markdown
# name of pipeline

description of the pipeline

## Task 1

description

- begin_if: <trigger condition>
- actions:
  - action1
  - action2
- if_fail: <hook action>
- if_succeed: <hook action>

## Task 2

...
```

A pipeline can start with a `config` code block, before the top-level
heading. The block defines model profiles for agent actions (see "Model
profiles" below):

````markdown
```config
local-qwen: model=qwen3.5:0.8b, base_url=http://localhost:11434/v1
openai-gpt5: model=gpt-5, base_url=https://api.openai.com/v1, api_key=OPENAI_API_KEY
```

# name of pipeline
````

A task's id is its heading text, so `## Build` is referred to as `task(Build)`.
Task ids must be unique. See `tests/fixtures/example_job.md` for a full example, and
`jobs/sample_api_calls.md` for a job that uses every action type.
`jobs/sample_agents.md` shows a `websearch` action and a `summarizer` agent.
`jobs/sample_foreach.md` shows the `foreach` action prefix.
`jobs/scripts/` has helper scripts that the sample jobs call, and
`tests/sample_scripts.rs` tests them.

Each action is one line of words separated by spaces. The first word selects
the type of action, and the other words are its parameters:

| Action | What it does | Output |
| --- | --- | --- |
| `bash <command> [args...]` | Runs a command in bash. | stdout |
| `api_call <verb> <url> [options...]` | Sends an HTTP request. | The response body |
| `download <url> [options...]` | Downloads a file. | The file path |
| `tavily_search [options...] [query...]` | Searches the web with Tavily. | The response JSON |
| `websearch topic="<text>" [options...]` | Searches the web for one topic. | The response JSON, unchanged |
| `agent:<type> model=<profile> [options...]` | Runs a `coder` or `summarizer` agent. | The final reply of the model |
| `foreach <action>` | Runs the action once for each input line. | All outputs, one after the other |
| `mock` | Does nothing. | The input, unchanged |
| `mock fail` | Always fails. | None |

For example: `bash npm ci`, `api_call GET https://example.com`,
`websearch topic="latest Rust release"` and
`agent:coder model=local-qwen task="Fix the failing test."`.

The `api_call` verb is GET, POST, PUT, DELETE, PATCH or HEAD. Its options
are `payload=<text>` and `input_file=<path>`, which set the request body.
The options of `download` are `mime=<type>` and `output_dir=<dir>`. To send
`Authorization: Bearer <token>`, add `auth=bearer:<token>` to an `api_call`
or `download` action. The log shows the token as `***`. The sections below
explain the other actions.

Each action gets the output of the action before it as its input. The first
action of a task gets the output of the last task that succeeded.

Tasks run in sequence by default. A `begin_if` trigger condition can make a task
depend on another task's result, or run in parallel with it:

- prev task succeed (default)
- prev task fail
- prev task succeed or fail
- task(id) succeed
- task(id) fail
- task(id) succeed or fail
- task(id) begin

The `if_fail` and `if_succeed` hooks run when a task finishes:

- `retry=N`: retry the failing task, up to N times
- `die`: stop and skip all remaining tasks
- `restart`: restart the whole job from the first task
- `call_api(...)`: make an API call
- `write_file(...)`: write the output to a file
- `write_error_file(...)`: write stderr to a file

Atama evaluates the job into a graph, using each task's `begin_if` condition to
work out its dependencies.

### The `foreach` prefix

Put `foreach` before an action to run the action once for each line of its
input. Each run gets one line, without its newline, as its input. The
action after `foreach` is a normal action, for example
`foreach bash ./my_script.sh` or `foreach agent:summarizer model=local-qwen`.

```markdown
## List items

- actions:
  - bash printf %s\n apple banana cherry

## Describe each item

- actions:
  - foreach bash awk {print"[item:",$0"]","has",length($0),"characters"}
```

The runs follow the order of the lines. Atama skips blank lines. The output
of `foreach` is the outputs of all runs, one after the other. If an output
does not end with a newline, Atama adds one, so that the output of each
item starts on a new line. An input without lines runs the action zero
times, and the output is empty.

If one run fails, `foreach` stops and fails, and the error names the item,
as in "`foreach` failed at item 2 (`banana`): ...". A `foreach` action
cannot run another `foreach` action. With `--debug`, each run writes its
own debug output.

## Agents

An `agent:coder` or `agent:summarizer` action runs an agent loop. The agent
loop sends the task to a chat model, runs the tools that the model asks for,
and sends the results back. When the model replies without a tool call, that
reply is the output of the action.

The agent works with any model on a server with an OpenAI compatible
`/chat/completions` endpoint, for example OpenAI, Ollama, vLLM or LM Studio.
The model must support tool calls.

Each word after `agent:<type>` must be an option, as in `key=value`.
`task="<text>"` sets the task, the instruction for the model. The output of
the previous action or task is added below the task as its input. If an
action has no task and no input, it fails. A word that is not an option,
or an unknown option, makes the action fail.

Put a value in double quotes if it has spaces, as in
`task="Fix the failing test."`. Atama removes the quotes, and keeps the
text between them unchanged. To put a double quote in a quoted value,
write `\"`. To put a backslash in a quoted value, write `\\`.

These options set up every agent:

- `model=<profile>` selects a model profile from the `config` block. You
  must set it.
- `max_steps=<n>` sets the most model replies in one run. The default is 10.
  If the model still calls tools after that, the action fails.

### Model profiles

A model profile is a name for one model on one server. You define the
profiles in a `config` code block at the start of the job file, before the
`# <name>` heading. Each line defines one profile:

```text
<name>: model=<id>, base_url=<url>[, api_key=<ENV_VAR>]
```

- `<name>` is the name that agent actions use, as in `model=local-qwen`. It
  is one word.
- `model=<id>` is the model identifier that the server knows, for example
  `qwen3.5:0.8b` or `gpt-5`. You must set it.
- `base_url=<url>` is the base URL of the server, for example
  `http://localhost:11434/v1`. You must set it.
- `api_key=<ENV_VAR>` is the name of the environment variable that holds
  the API key, for example `OPENAI_API_KEY`. Atama sends the key as a bearer
  token. Leave it out if the server needs no key, for example Ollama.

The job file holds only the name of the variable, never the key. If a
profile names a variable that is not set, an action that uses the profile
fails before it asks the model. Several actions can use the same profile.

Atama reads the profiles when it reads the job file. If a line is not a
valid profile, or an action names a profile that the block does not
define, the job does not start. The job can have only one `config` block.

### The `summarizer` agent

A `summarizer` agent summarizes its input, which is the output of the
previous action or task. The summary keeps the key information of the
input. If the input names numbers, quantities, dates, names or concepts
that are important to its key matter, the summary keeps them. The summary
is the output of the action. A `summarizer` agent has no tools, so it
sends one request to the model. It has these options, in addition to the
model options above:

- `expertise="<text>"` sets the field of the reader, for example
  `expertise="molecular biology"`. The summary uses the terms and the point
  of view of this field. The default is `general`.
- `max_output="<size>"` sets how long the summary can be. The size is a part
  of the input in percent, as in `60%`, or a number of sentences, words or
  paragraphs, as in `"3 sentences"`. If you do not set it, the model makes
  the summary as short as it can without losing key information.

`task=` is the instruction. If the action has no `task=`, the instruction is
"Summarize the input." For a size in percent, Atama counts the words of the
input and tells the model how many words it can use, because a model cannot
measure a percent of a text well.

```markdown
- actions:
  - bash cat ./notes.md
  - agent:summarizer model=local-qwen expertise="software engineering" max_output="3 sentences"
```

The action fails before it asks the model if the input is empty.
`expertise=` and `max_output=` work only with `agent:summarizer`.

### Agent tools

Each type of agent has its own tools:

- `coder` has `bash`, which runs a command in the directory where you start
  Atama.
- `summarizer` has no tools.

Atama cuts a tool result after 20,000 characters.

To see what an agent does, add `--debug` before the job file. Atama then
writes these blocks to stderr for each agent action:

1. The configuration: the profile name, the model, the base URL,
   `max_steps`, and whether an API key is set. Atama never writes the key itself. For a `summarizer`
   agent, it also has `expertise` and `max_output`.
2. For each step, the request: the number of messages, the tools, and the
   messages that are new since the last request, as pretty JSON.
3. The model's reply, as pretty JSON.
4. One block per tool call, with the tool name, the call id, the arguments
   as pretty JSON, and the result: `ok` and its length, or the error.

Each block starts with a line such as
`=== agent:coder step 1: tool call 1 of 2: bash (id call-1) ===`.
The task log stays on stdout, so `2> debug.log` saves the debug output in a
separate file. `--debug` has no effect in a dry run, because a dry run runs
no agents.

If you use `agent:coder`, the model can run any command with your
permissions. Use it only in a directory and with a model that you trust.

## Tavily search

Atama has a small client for the
[Tavily search API](https://docs.tavily.com/documentation/api-reference/endpoint/search).
It sends `POST https://api.tavily.com/search` directly, without an SDK. You
can use Tavily in two ways: with the `websearch` action, or with the
`tavily_search` action. Both return the Tavily response as JSON.

Each search uses Tavily API credits. To use Tavily, set your API key before
you run Atama:

```sh
export TAVILY_API_KEY=<your key>
```

Atama reads the key only from `TAVILY_API_KEY`. It sends the key as a
bearer token, and it never writes the key in the job file or the log.

### The `websearch` action

A `websearch` action sends one Tavily search for a topic. It asks no model.
Its output is the JSON response body from Tavily, exactly as Tavily sends
it. A later action can read the JSON, or a `summarizer` agent can analyse
it. The action ignores its input. It has these options:

- `topic="<text>"` sets what to search for. You must set it.
- `max_results=<n>` sets the most search results, from 1 to 20. The default
  is 5.
- `fetch_content=true` also gets the content of each result page, as
  markdown. Atama then sends `include_raw_content=markdown` to Tavily, and
  each result in the response has a `raw_content` field. The default is
  `false`.
- `base_url=<url>` sets the server. The default is `https://api.tavily.com`.

```markdown
- actions:
  - websearch max_results=5 fetch_content=true topic="latest stable Rust release"
  - bash tee ./search.json
```

Atama reads the options when it reads the job file. If the topic is not
set, an option is unknown, or a value is invalid, the job does not start.
The action fails if `TAVILY_API_KEY` is not set or the search fails.

With `--debug`, the action writes its options and the Tavily response, as
pretty JSON, to stderr. If the response is not JSON, Atama writes it as it
is. The output of the action is always the response without changes.

`websearch` was `agent:websearch` before. A job file that still has
`agent:websearch` does not start, and the error tells you to use
`websearch`.

### The `tavily_search` action

The action sends one search and needs no model. The words that are not
options are the query. If there are none, the output of the previous action
is the query. The output is the Tavily response as JSON:

```markdown
## Search

- actions:
  - tavily_search max_results=3 topic=news latest stable Rust release
  - bash jq -r .results[].url
```

These options change the search. If you do not set an option, Tavily uses
its default:

- `max_results=<n>` sets the most results.
- `search_depth=<depth>` sets the depth, for example `basic` or `advanced`.
- `topic=<topic>` sets the kind of search, for example `general`, `news` or `finance`.
- `include_answer=<value>` asks for a short answer: `true`, `false`, `basic` or `advanced`.
- `include_raw_content=<value>` asks for the content of each result page: `true`, `false`, `markdown` or `text`.
- `base_url=<url>` sets the server. The default is `https://api.tavily.com`.

The action fails if `TAVILY_API_KEY` is not set, the query is empty, or
Tavily responds with an error status.

## Building

Atama is a Rust project. You need a Rust toolchain with Cargo
(install it from <https://rustup.rs>).

```sh
# Debug build, output in target/debug/atama
cargo build

# Optimized build, output in target/release/atama
cargo build --release
```

## Running

Atama takes one required argument, the path to a job config markdown file:

```sh
# Build and run in one step (arguments after -- go to atama)
cargo run -- job_config_file.md

# Example: run the sample API job (it needs a server on localhost:8080)
cargo run -- jobs/sample_api_calls.md

# Or run a built binary directly
./target/release/atama job_config_file.md

# Print the job and log each action, but run nothing
cargo run -- --dry-run jobs/sample_api_calls.md

# Also print what each agent sends to its model and gets back (to stderr)
cargo run -- --debug jobs/sample_agents.md

# Show usage
cargo run -- --help
```

Atama runs the actions in the directory where you start it, so relative
paths such as `./output` are relative to that directory. The log shows each
action with a `$` prefix and each line of its output with a `|` prefix. Each
line of a task starts with the local date and time:

```text
2026-10-24 09:15:02 [Prepare] start
2026-10-24 09:15:02 [Prepare] $ bash mkdir -p ./output/agents
2026-10-24 09:15:02 [Prepare] succeeded
```

### Dry run

A dry run shows what a job will do, but it runs no actions. Use it to check
a new job file before you run it for real.

To do a dry run, add `--dry-run` before the job file:

```sh
cargo run -- --dry-run jobs/sample_api_calls.md

# Or with a built binary
./target/release/atama --dry-run jobs/sample_api_calls.md
```

In a dry run, Atama does these steps:

1. It parses the job file and reports any error with its line number.
2. It prints the job summary.
3. It logs each action with a `$` prefix, but it does not run the action.

In a dry run, every action succeeds and passes its input on unchanged. Thus
the log shows the tasks that run when every action succeeds. A dry run does
not test the failure paths: `mock fail` succeeds, and `if_fail` hooks and
tasks that start only on a failure, such as `begin_if: task(A) fail`, do not
run. A dry run needs no server, and it
does not create, change or delete files.

## Mock Server

When testing Atama with mock API calls, you can install and start a mock server with the help from [siga](https://github.com/tao-pr/siga). It allows you to create a mock API server from OpenAPI v3 spec with mock authentication and authorization.

```sh
cd siga & cargo build --release
./target/release/siga --api tests/data/petstore.yaml
```

## Testing

```sh
# Run all tests: unit tests in src/ and integration tests in tests/
cargo test

# Run only the tests whose name contains a filter string
cargo test <name_filter>

# Run only the integration tests in one file, e.g. tests/cli.rs
cargo test --test cli
```

Unit tests live next to the code in `src/`, inside a `#[cfg(test)] mod tests`
block. Integration tests go in `tests/`, one file per test crate.

The tests never call a real model or the real Tavily API. They use a local
mock server, so `cargo test` needs no network and uses no Tavily credits,
even if `TAVILY_API_KEY` is set.
