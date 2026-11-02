# Atama

Atama runs automation pipelines that you write in markdown. You describe the
tasks in a markdown file, and Atama runs them. You can mix agent-based task with non-agent tasks.

## Pipeline format

The top-level heading names the pipeline. Each second-level heading is a
task:

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

A `config` block before the top-level heading defines model profiles for
agent actions (see "Model profiles"):

````markdown
```config
local-qwen: model=qwen3.5:0.8b, base_url=http://localhost:11434/v1
openai-gpt5: model=gpt-5, base_url=https://api.openai.com/v1, api_key=OPENAI_API_KEY
```

# name of pipeline
````

The task id is its heading text, so `## Build` is `task(Build)`. Task ids
must be unique.

Examples:

- `tests/fixtures/example_job.md`: a full job.
- `jobs/sample_api_calls.md`: every action type.
- `jobs/sample_agents.md`: a `websearch` action and a `summarizer` agent.
- `jobs/sample_foreach.md`: the `foreach` prefix.
- `jobs/sample_agent_with_structured_output.md`: an agent with
  `output_schema=`.
- `jobs/sample_coder.md`: a `coder` agent that writes a Python program, and
  a `summarizer` agent that explains it.

The sample jobs call helper scripts in `jobs/scripts/`.
`tests/sample_scripts.rs` tests them.

## Actions

An action is one line of words. The first word is the action type, and the
other words are its parameters:

| Action | What it does | Output |
| --- | --- | --- |
| `bash <command> [args...]` | Runs a command in bash. | stdout |
| `bash-sandbox <command> [args...]` | Runs a command in bash in the macOS sandbox. | stdout |
| `api_call <verb> <url> [options...]` | Sends an HTTP request. | The response body |
| `download <url> [options...]` | Downloads a file. | The file path |
| `tavily_search [options...] [query...]` | Searches the web with Tavily. | The response JSON |
| `websearch topic="<text>" [options...]` | Searches the web for one topic. | The response JSON, unchanged |
| `agent:<type> model=<profile> [options...]` | Runs a `coder` or `summarizer` agent. | The final reply of the model |
| `read_file [path]` | Reads a text file. Without a path, the input is the path. | The content of the file, unchanged |
| `write_file <path>` | Writes the input to a file. | The input, unchanged |
| `foreach <action>` | Runs the action once for each input line. | All outputs, one after the other |
| `mock` | Does nothing. | The input, unchanged |
| `mock fail` | Always fails. | None |

For example: `bash npm ci`, `api_call GET https://example.com`,
`websearch topic="latest Rust release"` and
`agent:coder model=local-qwen task="Fix the failing test."`.

Each action gets the output of the action before it as its input. The first
action of a task gets the output of the last task that succeeded.

### `actions-parallel`

Use `- actions-parallel:` in place of `- actions:` to run the actions of a
task at the same time. Each action gets the same input from the previous task. The output from all parallel actions are not passed to the next task.

```markdown
## Notify

- actions-parallel:
  - write_file ./output/report.txt
  - api_call POST https://example.com/hook
```

`- actions-parallel:max=2` runs at most 2 actions at the same time. The
others wait until an action finishes. Without `max=`, all actions run at the
same time.

The log shows each action and its output in the order of the task, after
all actions finish. If an action fails, the task fails with the errors of
all failed actions. A retry runs all actions again.

### `api_call` and `download`

The `api_call` verb is GET, POST, PUT, DELETE, PATCH or HEAD.
`payload=<text>` or `input_file=<path>` sets the request body. `download`
takes `mime=<type>` and `output_dir=<dir>`.

To send `Authorization: Bearer <token>`, add `auth=bearer:<token>`. The log
shows the token as `***`.

### `bash-sandbox`

`bash-sandbox` runs a command as `bash` does, but inside the macOS sandbox
(`sandbox-exec`). The command has no network, and it can read and write
files only in the working directory. It cannot run `sudo`, `su` or `login`,
and it stops if Atama runs as root.

The command can also read the system directories that it needs to run
programs, such as `/usr` and `/opt/homebrew`. The home directory is closed,
so `PATH` has only system directories, and programs such as `pyenv` shims
do not run. The profile is in `src/actor/bash_sandbox.sb`.

`bash-sandbox` works only on macOS. A tool that needs a temporary directory
outside the working directory can fail or print warnings.

### `read_file`

`read_file ./notes.txt` reads `./notes.txt`, and `read_file "my notes.txt"`
reads a path with spaces. Without a path, the input is the path. The output
keeps every line of the file.

The input must be one line. To read one file for each line, use
`foreach read_file`. The file must be UTF-8 text.

### `write_file`

`write_file ./output/result.txt` writes its input to the file, unchanged,
and passes the input on as its output. It makes missing directories and
replaces an existing file. The `write_file(...)` hook is a different thing:
it runs after a task.

### `foreach`

`foreach <action>` runs the action once for each line of its input, with
that line as the input. Atama skips blank lines. The output is the outputs
of all runs, in order.

```markdown
## List items

- actions:
  - bash printf %s\n apple banana cherry

## Describe each item

- actions:
  - foreach bash awk {print"[item:",$0"]","has",length($0),"characters"}
```

If one run fails, `foreach` stops, and the error names the item, as in
"`foreach` failed at item 2 (`banana`): ...". A `foreach` cannot run another
`foreach`.

## Triggers and hooks

Tasks run in sequence by default. A `begin_if` condition makes a task wait
for another task, or run in parallel with it:

- prev task succeed (default)
- prev task fail
- prev task succeed or fail
- task(id) succeed
- task(id) fail
- task(id) succeed or fail
- task(id) begin

The `if_fail` and `if_succeed` hooks run when a task finishes:

- `retry=N`: retry the failing task, up to N times.
- `die`: stop and skip all remaining tasks.
- `restart`: restart the job from the first task.
- `call_api(...)`: make an API call.
- `write_file(...)`: write the output to a file.
- `write_error_file(...)`: write stderr to a file.

Atama makes a graph of the tasks from their `begin_if` conditions.

## Agents

An `agent:coder` or `agent:summarizer` action sends its task to a chat
model and runs the tools that the model asks for. The first reply without a
tool call is the output.

The model must support tool calls. It must be on a server with an OpenAI
compatible `/chat/completions` endpoint, such as OpenAI, Ollama, vLLM or LM
Studio.

Each word after `agent:<type>` is a `key=value` option. An unknown option
or a word without `=` makes the action fail. Put a value with spaces in
double quotes, as in `task="Fix the failing test."`, and write `\"` for a
quote and `\\` for a backslash.

Options of every agent:

- `model=<profile>`: the model profile. You must set it.
- `task="<text>"`: the instruction. The input goes below it. The action
  fails if it has no task and no input.
- `max_steps=<n>`: the most model replies in one run. The default is 10.
- `thinking=false`: sends `"reasoning_effort": "none"`, which turns off the
  thinking of the model on Ollama. Other servers can reject it.
- `output_schema=<name|path>`: makes the output a JSON value that matches a
  schema (see "Output schemas").

If you use `agent:coder`, the model can run any command with your
permissions. Use it only in a directory and with a model that you trust.

### Model profiles

A model profile is a name for one model on one server. Define each profile
on one line of the `config` block:

```text
<name>: model=<id>, base_url=<url>[, api_key=<ENV_VAR>]
```

- `<name>`: one word that agent actions use, as in `model=local-qwen`.
- `model=<id>`: the model identifier on the server, as in `qwen3.5:0.8b`.
  You must set it.
- `base_url=<url>`: the server URL, as in `http://localhost:11434/v1`. You
  must set it.
- `api_key=<ENV_VAR>`: the environment variable with the API key. Leave it
  out for a server without a key, such as Ollama.

The job file never holds the key. If the variable is not set, the action
fails before it asks the model. If a profile is invalid or unknown, the job
does not start.

### Output schemas (optional)

`output_schema=` makes the output of an agent JSON that matches a JSON
schema. The next action can read it, for example with `bash jq .title`. The
value is one of these:

- The path of a schema file, as in `output_schema=./schemas/article.json`.
- The name of a schema block in the job file, as in `output_schema=article`.

A schema block comes before the `# <name>` heading. Its fence is
`json:<name>`:

````markdown
```json:article
{
  "type": "object",
  "properties": {
    "title": {"type": "string"},
    "points": {"type": "array", "items": {"type": "string"}}
  },
  "required": ["title", "points"]
}
```

# Summarize the news

## Summary

- actions:
  - agent:summarizer model=local-qwen output_schema=article
````

The job does not start if a block is not a valid JSON schema, if a name is
used twice, or if an action names an unknown schema. Atama reads a schema
file when the action starts, so an earlier task can write it. A schema
cannot use a `$ref` to a URL.

Atama sends the schema in `response_format` and in the system prompt. It
also checks each reply, because some servers ignore `response_format`. If a
reply does not match, the model gets the errors and tries again, up to
`max_steps` replies.

### The `summarizer` agent

A `summarizer` agent summarizes its input and keeps the key facts, such as
numbers, dates and names. It has no tools, and it fails if the input is
empty. The default task is "Summarize the input."

It has two more options:

- `expertise="<text>"`: the field of the reader, as in
  `expertise="molecular biology"`. The default is `general`.
- `max_output="<size>"`: the most that the summary can be, as in `60%` or
  `"3 sentences"`. The unit can also be words or paragraphs. Without it,
  the summary is as short as it can be.

```markdown
- actions:
  - read_file ./notes.md
  - agent:summarizer model=local-qwen expertise="software engineering" max_output="3 sentences"
```

### Agent tools

A `coder` agent has `bash`, which runs a command in the directory where you
start Atama. A `summarizer` agent has no tools. Atama cuts a tool result
after 20,000 characters.

### Debug output

Add `--debug` to see what an agent does. Atama writes the configuration,
each request, each reply and each tool call to stderr, as pretty JSON. It
never writes the API key.

Each block starts with a line such as
`=== agent:coder step 1: tool call 1 of 2: bash (id call-1) ===`. To save
the debug output in a file, add `2> debug.log`.

## Tavily search

The `websearch` and `tavily_search` actions search the web with the
[Tavily search API](https://docs.tavily.com/documentation/api-reference/endpoint/search).
Both return the Tavily response as JSON, and each search uses Tavily API
credits. Set your key first:

```sh
export TAVILY_API_KEY=<your key>
```

Atama reads the key only from `TAVILY_API_KEY`, and it never writes the key
in the log.

### The `websearch` action

A `websearch` action searches one topic and ignores its input. Its options:

- `topic="<text>"`: what to search for. You must set it.
- `max_results=<n>`: from 1 to 20. The default is 5.
- `fetch_content=true`: also gets each result page as markdown, in
  `raw_content`. The default is `false`.
- `base_url=<url>`: the server. The default is `https://api.tavily.com`.

```markdown
- actions:
  - websearch max_results=5 fetch_content=true topic="latest stable Rust release"
  - write_file ./search.json
```

A missing topic or a bad option stops the job before it starts. With
`--debug`, the action writes its options and the response to stderr. The
old name `agent:websearch` is an error.

### The `tavily_search` action

The words that are not options are the query. Without query words, the
input is the query.

```markdown
- actions:
  - tavily_search max_results=3 topic=news latest stable Rust release
  - bash jq -r .results[].url
```

Options that you do not set use the Tavily default:

- `max_results=<n>`: the most results.
- `search_depth=<depth>`: `basic` or `advanced`.
- `topic=<topic>`: `general`, `news` or `finance`.
- `include_answer=<value>`: `true`, `false`, `basic` or `advanced`.
- `include_raw_content=<value>`: `true`, `false`, `markdown` or `text`.
- `base_url=<url>`: the server. The default is `https://api.tavily.com`.

## Building

You need Rust with Cargo, from <https://rustup.rs>.

```sh
# Debug build, output in target/debug/atama
cargo build

# Optimized build, output in target/release/atama
cargo build --release
```

## Running

Atama takes the path of a job file:

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

Atama runs the actions in the directory where you start it. The log shows
each action with `$` and each output line with `|`:

```text
2026-10-24 09:15:02 [Prepare] start
2026-10-24 09:15:02 [Prepare] $ bash mkdir -p ./output/agents
2026-10-24 09:15:02 [Prepare] succeeded
```

### Dry run

`--dry-run` parses the job, reports errors with their line numbers, and
logs each action without running it. Every action succeeds and passes its
input on, so `if_fail` hooks and failure triggers do not run. A dry run
needs no server and does not change files.

```sh
cargo run -- --dry-run jobs/sample_api_calls.md

# Or with a built binary
./target/release/atama --dry-run jobs/sample_api_calls.md
```

## Mock Server

To test API calls, you can run a mock server with
[siga](https://github.com/tao-pr/siga). It makes a mock API from an OpenAPI
v3 spec, with mock authentication and authorization.

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

Integration tests are in `tests/`, one file per test crate. The tests never
call a real model or the real Tavily API, so `cargo test` needs no network.
