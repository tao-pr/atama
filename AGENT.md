# Guideline for Agent

This project is developed in Rust. It should follow the official structure and convention.

## Generic Project Structure

- Cargo.lock
- Cargo.toml
- src/
 - *.rs
 - main.rs
 - bin/
   - foo-executable.rs
   - multi-executables/
     - foo-executable.rs
     - bar-executable.rs
- tests/
  - some-tests.rs
  - another-tests.rs
  - multi-file-tests
    - main.rs
    - test1.rs


This project structure allows us to conveniently package it into a Cargo package.

- The default executable file is src/main.rs
- The default library file is src/lib.rs

## Functionalities

The project Atama is an automation pipeline as code. It allows users to create 
a simple pipeline to run with or without agent of their choice in markdown format.
The conceptual structure of the pipeline looks like:

````markdown
```config
<profile name>: model=<id>, base_url=<url>, api_key=<ENV_VAR_NAME_OF_API_KEY>
```

# name of pipeline

description of the pipeline

## Task 1

description

- begin_if: <trigger condition>
- actions:
  - action1
  - action2
  - action3
- if_fail: <hook action>
- if_succeed: <hook action>

## Task 2

description

- begin_if: <trigger condition>
- actions:
- if_fail: <hook condition>
- if_succeed: <hook action>

## Task 3

## Last task

description

````

The optional `config` block defines model profiles for agent actions (see
"Model, debug output and tests" below).

### Parallel actions

A task can have `- actions-parallel:` in place of `- actions:`. Its actions
run at the same time, and each gets the input of the task. The task passes
no output on, so the next task always gets an empty input. `Task::parallel`
marks such a task. `- actions-parallel:max=<n>` sets
`Task::max_parallel`, the most actions that run at the same time
(`parallel_limit` in `src/parser.rs` reads it). `ActionRunner::run_parallel`
runs the actions: its default runs them one after the other, and
`ActorRunner` starts `max` threads, or one per action, that take the next
action until none is left. `Executor::run_parallel_actions` logs the outputs in the order of
the task, after all actions finish. The error of a failed action is on a
`! ` line right under the command of that action.

### Trigger conditions

The tasks are generally executed in sequence unless the `begin_if` hook instructs it to begin in parallel with other tasks. The possible trigger conditions are:

- prev task succeed (default if not set otherwise)
- prev task fail
- prev task succeed or fail
- task(id) succeed
- task(id) fail
- task(id) succeed or fail
- task(id) begin 

### Hook conditions

Hooks are executed when the task fails or succeeds (or other criteria are met).
Supported hooks are:

- retry=N (retry the current failing task, it will retry max N times)
- die (this will ignore all other unexecuted jobs)
- restart (this will restart the whole job from the first job)
- call_api(...) make an api call. The parameter will be discussed later in the code.
- write_file(...) write the output to a file
- write_error_file(...) write the stderr to a file

## Execution of the job

The job is evaluated into a graph structure. The dependencies of tasks are evaluated 
with the begin hook.

## Actors

An actor is the code that carries out one action of a task. The `Actor` enum
is in `src/actor.rs`. The parser makes one actor from each action line. The
first word of the line selects the variant. `src/actor/display.rs` writes an
actor back as an action line.

Each actor gets the output of the previous action as its input, and returns
its own output. The main actor types are:

- `BashScript` (`bash <cmd> [args...]`) runs a command in bash. The output is stdout.
- `BashSandbox` (`bash-sandbox <cmd> [args...]`) runs a command in bash inside `sandbox-exec`, on macOS only. The profile, `src/actor/bash_sandbox.sb`, denies the network, `sudo`, `su` and `login`, and files outside the working directory, except the system directories that programs need. `run_bash_sandbox` in `src/actor.rs` sets `PATH` to `SANDBOX_PATH` and stops if the user is root.
- `ApiCall` (`api_call <verb> <url> ...`) sends an HTTP request. The output is the response body.
- `DownloadFromUrl` (`download <url> ...`) saves a file. The output is the file path.
- `AgentTool` (`agent:<type> ...`) runs an agent, `coder` or `summarizer`. The output is the final reply of the model.
- `TavilySearch` (`tavily_search ...`) searches the web with the Tavily API (`src/tavily.rs`). The output is the response JSON.
- `WebSearch` (`websearch topic=...`) searches one topic with Tavily (`src/websearch.rs`). The output is the response body, unchanged.
- `ForEach` (`foreach <action>`) runs the inner actor once for each line of its input that is not blank, with the line as the input. The output is the outputs of all runs, in order, each one ending with a newline. It stops at the first run that fails. `for_each` in `src/actor.rs` runs it, and `parse_action` in `src/parser.rs` also gives the profile to an agent inside it.
- `ReadFile` (`read_file [path]`) reads a text file. Without a path, the input, trimmed, is the path. It fails on an input with more than one line. The output is the content of the file, unchanged. `read_file` in `src/actor.rs` runs it.
- `ListFiles` (`list_files [dir=<path>] [pattern=<glob>]`) lists the files of one directory whose names match a glob. The default pattern is `*`. Without `dir=`, the input, trimmed, is the directory. It does not go into subdirectories. The output is one path per line, sorted. `src/actor/list_files.rs` runs it and has the glob matcher.
- `WriteFile` (`write_file <path>`) writes its input to a file, and makes the missing parent directories. The output is the input, unchanged, as with `tee`. `write_file` in `src/actor.rs` runs it.
- `MockPlaceholder` (`mock`) passes its input on unchanged. `MockFail` (`mock fail`) always fails.

`ApiCall` and `DownloadFromUrl` can take an `Auth`. A `Passer` (`src/passer.rs`)
reads a secret value from data, a file, an environment variable, or the
stdout of a command.

## Agent actor

The `AgentTool` actor runs an agent loop. The only loop now is
`AgentToolLoop` in `src/agent_loop_openai_compat.rs`. It works with a server
that has an OpenAI compatible `/chat/completions` endpoint, for example
OpenAI, Ollama or vLLM.

The loop sends the task to the model through the `ChatClient` trait. If the
model asks for tool calls, the loop runs them and sends the results back. If
the model replies without a tool call, that reply is the output.
`max_steps` limits the number of model replies.

The code of the loop is in four files:

- `src/agent_loop_openai_compat.rs` has `AgentToolLoop`, the messages, and the `Tool` and `ChatClient` traits.
- `src/agent_loop_openai_compat/agent_options.rs` has the options of an agent action: `AgentConfig`, `WebSearchOptions`, `SummarizerOptions`, `MaxOutput`, and the defaults and environment variable names of the options.
- `src/agent_loop_openai_compat/openai.rs` has `OpenAiClient`, the HTTP client, and the JSON format of requests and replies.
- `src/agent_loop_openai_compat/tools.rs` has the `Tool` implementations and the tools of each `AgentType`.

### Options of an agent action

Each param of an agent action is an option. Every agent needs
`model=<profile>` and takes `task=`, `max_steps=`, `thinking=` and
`output_schema=`, and a
`summarizer` agent also takes
`expertise=` and `max_output=`. A `coder` agent fails on these two options
(`only_for`). A value can be in double quotes, so that it can have spaces,
as in
`task="Fix the failing test."`. The parser (`split_quoted` in
`src/parser.rs`) splits the params of an `agent:` line at whitespace, but
not inside quotes, and removes the quotes. Inside quotes, `\"` is a quote and
`\\` is a backslash. Then `Values::read` in `agent_options.rs` reads each
param for `AgentConfig::from_params`. It fails on a word without `=` and on
an unknown key. `src/actor/display.rs` puts the quotes back when it writes
an agent action as a line. The `websearch` action uses `split_quoted` in
the same way.

### Steps of one agent run

`Actor::AgentTool` makes a loop with `AgentToolLoop::new` and calls
`run_loop`. One run has these steps:

1. Get the tools. `AgentToolLoop::new` gets the tools of the agent type from
   `tools::default_tools`. See "Tools of each agent type" below.
2. Read the configuration. `run_loop_with_env` reads the options of the
   action and the model profile with `AgentConfig::from_params`. `task=` becomes
   `AgentConfig::task`.
3. Make the task. For `summarizer`, `SummarizerOptions::task` puts the
   instruction, the expertise and the length limit into the task.
4. Start the conversation. `run_with` makes two messages: the system prompt
   of the agent type (`system_prompt`) and the user prompt (`user_prompt`).
   The user prompt is the task and the input of the action.
5. Make the request. `run_with` calls `ChatClient::chat` with all the
   messages and all the tools. `OpenAiClient::chat` makes the JSON body with
   `openai::request_body`, and sends `POST <base_url>/chat/completions`.
   The `tools` field of the body has the name, the description and the
   parameter schema of each tool.
6. Parse the reply. `openai::parse_reply` reads the text and the
   `tool_calls` of the first choice into a `Reply`. Each `ToolCall` has an
   id, the name of a tool, and the arguments as a JSON string.
7. Decide whether to stop. If the reply has no tool calls, `run_with`
   returns the text of the reply as the output. Else, `run_with` adds the
   reply to the conversation and goes on.
8. Call the tools. `run_with` calls the tools in the order of the reply.
   `call_tool` finds the tool with the same name, parses the arguments as
   JSON, and calls `Tool::call`. The result goes into the conversation as a
   tool message with the id of the call.
9. Go back to step 5. If the model still calls tools after `max_steps`
   replies, the run fails.

The model, not Atama, decides which tools to call and with which
arguments. The request has no `tool_choice` field, so the server uses
`auto`: the model can call any tool in the list, or no tool. Atama
controls the choice only with the tool list, the tool descriptions and the
system prompt.

A failed tool call does not stop the loop. If the model names a tool that
is not in the list, `call_tool` fails with "unknown tool". If the arguments
are not JSON, or the tool fails, the call also fails. In each case, the
model gets the error text as the tool result, and it can try again.

### Tools of each agent type

A tool is a type that implements the `Tool` trait. The trait has four
methods: `name`, `description`, `parameters` (a JSON schema) and `call`.
`tools::default_tools` gives each agent type its tools:

- `coder` gets `Bash` (`bash`).
- `summarizer` gets no tools. `request_body` then leaves out the `tools`
  field, so the model can only reply with text.

The tools run the same code as the actions of a job file. `bash` runs an
`Actor::BashScript`. If a result is
longer than `MAX_RESULT_CHARS` (20,000 characters), `tools::truncate` cuts
it.

The model of an agent can call only the tools that `default_tools` gives to
its agent type. For example, a `summarizer` agent cannot run `bash`. To give an
agent type a new tool, do these steps:

1. Write a type that implements `Tool` in `tools.rs`.
2. Add the tool to the match arm of the agent type in `default_tools`.
3. If the model must know when to use the tool, tell it in `system_prompt`.
4. Add tests to `tests/agent_loop_openai_compat.rs`.

Tests of the options are in `tests/agent_options.rs`. Tests that run the
loop are in `tests/agent_loop_openai_compat.rs`.

`AgentToolLoop::with_tools` replaces the tools of a loop. Tests use it to
give the loop mock tools.

### The `summarizer` agent

A `summarizer` agent summarizes the input of the action. It takes
`expertise=<text>` (default `DEFAULT_EXPERTISE`, `general`) and
`max_output=<size>`. `AgentConfig::from_params` reads them into
`SummarizerOptions` only for `summarizer`. `MaxOutput` is the parsed size:
`Percent`, `Sentences`, `Words` or `Paragraphs`. Without `max_output=`,
it is `None`, and the model chooses the length.

`SummarizerOptions::task` makes the task: the instruction, the expertise
and the length limit. The instruction is the value of `task=`, else
`DEFAULT_SUMMARIZER_PROMPT`. For `Percent`, the task also gives the number
of words that the percent allows. `user_prompt` adds the input after the
task. If the input is empty, `SummarizerOptions::task` fails, and the loop
does not ask the model.

Now, the expertise is only the name of a field in the task. A `TAOTODO`
comment on `SummarizerOptions::expertise` marks the plan to load domain
expertise as skills later.

### Model, debug output and tests

`thinking=false` makes `AgentConfig::thinking` `Some(false)`. Then
`openai::request_body` adds `"reasoning_effort": "none"`, which turns off
the thinking of the model on Ollama. Without `thinking=`, or with
`thinking=true`, the body has no `reasoning_effort`, and the server uses
its default.

The model comes from a model profile (`src/profile.rs`). A job file can
start with a ```` ```config ```` block, before the `# <name>` heading. Each
line is `<name>: model=<id>, base_url=<url>[, api_key=<ENV_VAR>]`, and
`ModelProfile::from_str` reads it. `parse_job` reads the block with
`profile::parse_profiles`, and `parse_action` uses `profile::resolve` to
put the profile that `model=<name>` names into `Actor::AgentTool::profile`.
An unknown name is a parse error. An action parsed with `str::parse` alone
has `profile: None`, and it fails when it runs.

`AgentConfig::from_params` takes the profile. It reads the model identifier
and the base URL from the profile, and the API key from the environment
variable that `api_key=` names. If that variable is not set, it fails. The
job file never holds a key. With `--debug`, the loop writes the profile
name, each request, reply and tool call to stderr. Each debug header names
the agent and its profile, as in `=== agent:coder (local-qwen3) step 1 ...`
(`AgentToolLoop::label`). The errors of the loop also name the profile. The header of each
request also has `reasoning_effort` if the request sends it.

`run_loop` reads the real environment. Tests call `run_loop_with_env` with a
fake environment.

In tests, use a mock `ChatClient` and mock tools, or a `ModelProfile` whose
`base_url` is a local mock server. Tests must not call a real model.

### Output schemas

`output_schema=<name|path>` makes the output of an agent a JSON value that
matches a JSON schema. The code is in `src/schema.rs`, and the
`jsonschema` crate checks the schemas.

- A value with only ASCII letters, digits, `_` and `-` is a name
  (`schema::is_name`). Any other value is a path.
- A job file can have ```` ```json:<name> ```` blocks before the
  `# <name>` heading. `parse_job` reads each block with
  `schema::parse_block`, which makes sure that it is JSON, a JSON object
  and a valid schema (`schema::check_schema`).
- `parse_action` uses `schema::resolve` to put the block that a name
  selects into `Actor::AgentTool::output_schema`. An unknown name is a
  parse error. The parser does not read a path.
- `AgentConfig::output_schema` is the raw value. `run_loop_with_env` uses
  `schema::load`, which takes the block from `AgentToolLoop::schema_block`
  for a name, and reads the file for a path.
- `openai::request_body` adds `response_format` with the schema, without
  `strict`. `run_with` adds `OutputSchema::instruction` to the system
  prompt, because some servers ignore `response_format`.
- `run_with` makes sure that the final reply matches the schema with
  `OutputSchema::check_reply`. If it does not match, the loop sends the
  errors to the model as a user message and goes on. Each try counts in
  `max_steps`.

Tests of the module are in `tests/schema.rs`. Tests set a schema on a loop
with `AgentToolLoop::output_schema`.

## Tavily client

`src/tavily.rs` is a small client for the Tavily search API. It sends
`POST <base_url>/search` with `ureq`, without an SDK. The API key comes only
from the `TAVILY_API_KEY` environment variable, and Atama sends it as a
bearer token.

- `SearchConfig::from_params` reads the options and the query of an action.
- `SearchConfig::request_body` makes the JSON body. It has only the options that are set.
- `SearchConfig::search` sends the request and returns the response body unchanged.

Two actions use the client, and the output of both is the response body.
The `tavily_search` action (`Actor::TavilySearch`) takes the options of
`SearchConfig` and a free-text query. The `websearch` action
(`Actor::WebSearch`) takes fewer options, in `src/websearch.rs`:

- `websearch topic=<text> [max_results=<n>] [fetch_content=<true|false>] [base_url=<url>]`.
- The parser reads the options into `WebSearchOptions` with `split_quoted`,
  `parse_options` and `WebSearchOptions::from_values`, so a wrong option
  stops the job before it starts.
- `WebSearchOptions::search` builds a `SearchConfig` and returns the body
  without changes. `fetch_content=true` sets `include_raw_content` to
  `markdown`. The action ignores its input and asks no model.
- `agent:websearch` was the old name. The parser rejects it with a message
  that names the `websearch` action.

Tests must not call the real Tavily API. Set `base_url=` to a local mock
server, as `tests/websearch.rs` and `tests/tavily.rs` do. Tests call
`WebSearchOptions::search` with a fake environment. Do not call `std::env::set_var`
in tests, because other tests read the environment in parallel. To test
with `TAVILY_API_KEY`, give the key to a child `atama` process, as
`tests/cli.rs` does.

## Test coverage

The project uses `cargo-llvm-cov` to measure test coverage. Test coverage is the part of the code that the tests run.
The tool shows which lines, functions and regions of `src/` the tests in `tests/` do not run.

To install the tool, run these commands once:

```sh
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --locked
```

To measure coverage, run these commands from the project root:

```sh
# Run all tests and show a coverage table for each file in src/
cargo llvm-cov

# Write a line-by-line report to target/llvm-cov/html and open it in the browser
cargo llvm-cov --html --open

# Measure coverage for one test file only, for example tests/actor.rs.
# Only the rows for the code that this file tests are useful.
cargo llvm-cov --test actor
```

When you add or change code, follow these rules:

1. Add tests in `tests/` for the new or changed code.
2. Run `cargo llvm-cov` before and after the change.
3. Make sure that line coverage for each changed file does not decrease.
4. If a line has no test, open the HTML report to find it.
