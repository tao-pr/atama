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

```markdown
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

```

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
- `ApiCall` (`api_call <verb> <url> ...`) sends an HTTP request. The output is the response body.
- `DownloadFromUrl` (`download <url> ...`) saves a file. The output is the file path.
- `AgentTool` (`agent:<type> ...`) runs an agent. The output is the final reply of the model.
- `TavilySearch` (`tavily_search ...`) searches the web with the Tavily API (`src/tavily.rs`). The output is the response JSON.
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

The code of the loop is in three files:

- `src/agent_loop_openai_compat.rs` has `AgentToolLoop`, `AgentConfig` and the `Tool` and `ChatClient` traits.
- `src/agent_loop_openai_compat/openai.rs` has `OpenAiClient`, the HTTP client, and the JSON format of requests and replies.
- `src/agent_loop_openai_compat/tools.rs` has the `Tool` implementations and the tools of each `AgentType`.

### Steps of one agent run

`Actor::AgentTool` makes a loop with `AgentToolLoop::new` and calls
`run_loop`. One run has these steps:

1. Get the tools. `AgentToolLoop::new` gets the tools of the agent type from
   `tools::default_tools`. See "Tools of each agent type" below.
2. Read the configuration. `run_loop_with_env` reads the options of the
   action with `AgentConfig::from_params`.
3. Search, only for `websearch`. `search_task` searches the topic with
   Tavily, and puts the prompt, the topic and the results into the task.
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
`tools::tools_for` gives each agent type its tools:

- `websearch` gets `FetchUrl` (`fetch_url`). It also gets `TavilySearch`
  (`tavily_search`), but only if `TAVILY_API_KEY` is set.
- `coder` gets `Bash` (`bash`).

The tools run the same code as the actions of a job file. `fetch_url` runs
an `Actor::ApiCall` with `GET`, and `bash` runs an `Actor::BashScript`.
`tavily_search` uses `SearchConfig` from `src/tavily.rs`. If a result is
longer than `MAX_RESULT_CHARS` (20,000 characters), `tools::truncate` cuts
it.

The model of an agent can call only the tools that `tools_for` gives to its
agent type. For example, a `websearch` agent cannot run `bash`. To give an
agent type a new tool, do these steps:

1. Write a type that implements `Tool` in `tools.rs`.
2. Add the tool to the match arm of the agent type in `tools_for`.
3. If the model must know when to use the tool, tell it in `system_prompt`.
4. Add tests to `tests/agent_loop_openai_compat.rs`.

`AgentToolLoop::with_tools` replaces the tools of a loop. Tests use it to
give the loop mock tools.

### The `websearch` agent

A `websearch` agent needs `topic="<text>"` and takes `max_results=<n>`
(default `DEFAULT_MAX_RESULTS`, 5). `AgentConfig::from_params` takes the
agent type, and it reads these two options into `WebSearchOptions` only for
`websearch`. For `coder`, it rejects them. `join_quoted` joins a quoted
option value that the parser split at its spaces. Before the first model
step, `AgentToolLoop::search_task` sends one Tavily search for the topic. It
gives the model the prompt, the topic and the results in one user message.
The prompt is the words that are not options, else `DEFAULT_WEBSEARCH_PROMPT`.

### Model, debug output and tests

The model comes from `model=`, `base_url=` and `max_steps=` in the action,
or from the `ATAMA_AGENT_MODEL`, `ATAMA_AGENT_BASE_URL` and
`ATAMA_AGENT_API_KEY` environment variables. With `--debug`, the loop writes
each request, reply and tool call to stderr.

`run_loop` reads the real environment. Tests call `run_loop_with_env` with a
fake environment, and `search_base_url` with a mock Tavily server.

In tests, use a mock `ChatClient` and mock tools. Tests must not call a real model.

## Tavily client

`src/tavily.rs` is a small client for the Tavily search API. It sends
`POST <base_url>/search` with `ureq`, without an SDK. The API key comes only
from the `TAVILY_API_KEY` environment variable, and Atama sends it as a
bearer token.

- `SearchConfig::from_params` reads the options and the query of an action.
- `SearchConfig::request_body` makes the JSON body. It has only the options that are set.
- `SearchConfig::search` sends the request and returns the response JSON.
- `format_results` turns the response JSON into short text for a model.

Three parts of Atama use the client. The `tavily_search` action
(`Actor::TavilySearch`) runs a search in a task, and its output is the
response JSON. A `websearch` agent searches its topic first with
`AgentToolLoop::search_task`, and gives the model the text of
`format_results`. The `tavily_search` tool (`TavilySearch` in `tools.rs`)
lets the model of a `websearch` agent search again, and it returns the
text of `format_results`.
`tools_for` gives the tool to `websearch` only if `TAVILY_API_KEY` is set.

Tests must not call the real Tavily API. Set `base_url=` or
`TavilySearch::new` to a local mock server. Do not call `std::env::set_var`
in tests, because other tests read the environment in parallel. To test
with `TAVILY_API_KEY`, give the key to a child `atama` process, as
`tests/cli.rs` does. A test that checks the tools of an agent must not
expect a fixed list, because the list depends on `TAVILY_API_KEY`.

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
