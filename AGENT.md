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

- `openai.rs` has `OpenAiClient`, the HTTP client, and the JSON format of requests and replies.
- `tools.rs` has the `Tool` implementations and the tools of each `AgentType`.
- `websearch` has the `fetch_url` tool, and `coder` has the `bash` tool.

The model comes from `model=`, `base_url=` and `max_steps=` in the action,
or from the `ATAMA_AGENT_MODEL`, `ATAMA_AGENT_BASE_URL` and
`ATAMA_AGENT_API_KEY` environment variables. With `--debug`, the loop writes
each request, reply and tool call to stderr.

In tests, use a mock `ChatClient` and mock tools. Tests must not call a real model.

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
