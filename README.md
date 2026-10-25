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

A task's id is its heading text, so `## Build` is referred to as `task(Build)`.
Task ids must be unique. See `tests/fixtures/example_job.md` for a full example, and
`jobs/sample_api_calls.md` for a job that uses every action type.
`jobs/sample_agents.md` shows two ways to configure an agent.

Each action is one line of words separated by spaces. The first word selects
the type of action, and the other words are its parameters:

- `bash <command> [arguments...]` runs the command in bash, for example
  `bash npm ci`. The output is stdout.
- `api_call <verb> <url> [payload=<text>] [input_file=<path>]` sends an HTTP
  request, for example `api_call GET https://example.com`. The verb is one of
  GET, POST, PUT, DELETE, PATCH or HEAD, in any case. The output is the
  response body.
- `download <url> [mime=<type>] [output_dir=<dir>]` downloads a file. The
  output is the path of the downloaded file.
- Add `auth=bearer:<token>` to an `api_call` or `download` action to send
  `Authorization: Bearer <token>`. The printed job and the log show the token
  as `***`.
- `tavily_search [max_results=<n>] [search_depth=<depth>] [topic=<topic>]
  [include_answer=<value>] [query...]` searches the web with the
  [Tavily search API](https://docs.tavily.com/documentation/api-reference/endpoint/search),
  for example `tavily_search max_results=3 latest stable Rust release`. The
  words that are not options are the query. If there are none, the input is
  the query. The output is the Tavily response as JSON. Set the API key in
  `TAVILY_API_KEY`. Atama sends it as a bearer token and never writes it
  in the log.
- `mock` does nothing and passes its input on as its output.
- `mock fail` always fails.
- `agent:<type> [model=<id>] [base_url=<url>] [max_steps=<n>] <task...>`
  runs an agent, for example
  `agent:websearch model=qwen3.5:0.8b base_url=http://localhost:11434/v1 summarize the news`.
  The type is `websearch` or `coder`. The output is the agent's final reply.
  See "Agents" below.

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

## Agents

An `agent:<type>` action runs an agent loop. The agent loop sends the task to
a chat model, runs the tools that the model asks for, and sends the results
back. When the model replies without a tool call, that reply is the output of
the action.

The agent works with any model on a server with an OpenAI compatible
`/chat/completions` endpoint, for example OpenAI, Ollama, vLLM or LM Studio.
The model must support tool calls.

The words of the action that are not options are the task. The output of the
previous action or task is added below the task as its input.

These options and environment variables set up the model:

- `model=<id>`, or `ATAMA_AGENT_MODEL`, sets the model. You must set one of them.
- `base_url=<url>`, or `ATAMA_AGENT_BASE_URL`, sets the server. The default is
  `https://api.openai.com/v1`.
- `ATAMA_AGENT_API_KEY` sets the API key, which Atama sends as a bearer token.
  A local server often needs no key. The key is never in the job file or
  the log.
- `max_steps=<n>` sets the most model replies in one run. The default is 10.
  If the model still calls tools after that, the action fails.

Each type of agent has its own tools:

- `websearch` has `fetch_url`, which sends a `GET` request to a URL. If
  `TAVILY_API_KEY` is set, it also has `tavily_search`, which searches the
  web with Tavily and returns the answer and the results as text.
- `coder` has `bash`, which runs a command in the directory where you start
  Atama.

Atama cuts a tool result after 20,000 characters.

To see what an agent does, add `--debug` before the job file. Atama then
writes these blocks to stderr for each agent action:

1. The configuration: the model, the base URL, `max_steps`, and whether an
   API key is set. Atama never writes the key itself.
2. For each step, the request: the number of messages, the tools, and the
   messages that are new since the last request, as pretty JSON.
3. The model's reply, as pretty JSON.
4. One block per tool call, with the tool name, the call id, the arguments
   as pretty JSON, and the result: `ok` and its length, or the error.

Each block starts with a line such as
`=== agent:websearch step 1: tool call 1 of 2: fetch_url (id call-1) ===`.
The task log stays on stdout, so `2> debug.log` saves the debug output in a
separate file. `--debug` has no effect in a dry run, because a dry run runs
no agents.

If you use `agent:coder`, the model can run any command with your
permissions. Use it only in a directory and with a model that you trust.

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
