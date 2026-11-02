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

An action is one line in the `- actions:` list of a task. The first word is
the action type, and the other words are its parameters. For example:

- `bash npm ci`, `api_call GET https://example.com`,
- `websearch topic="latest Rust release"` and
- `agent:coder model=local-qwen task="Fix the failing test."`.

Each action takes input from previous task/action, passes out its output to the next.

See [ACTIONS.md](ACTIONS.md) for full reference of all actions and parameters.

### `actions-parallel`

Use `- actions-parallel:` in place of `- actions:` to run the actions of a
task at the same time. Each action gets the same input from the previous
task. Atama does not pass the outputs of the parallel actions to the next
task, so the next task gets an empty input.

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
model. [ACTIONS.md](ACTIONS.md#agenttype) describes the options of each
agent. This section describes the parts of the job file that agents use.

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

### Debug output

Add `--debug` to see what an agent does. Atama writes the configuration,
each request, each reply and each tool call to stderr, as pretty JSON. It
never writes the API key.

Each block starts with a line such as
`=== agent:coder step 1: tool call 1 of 2: bash (id call-1) ===`. To save
the debug output in a file, add `2> debug.log`.

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
