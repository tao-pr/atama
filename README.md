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
- `mock` does nothing and passes its input on as its output.
- `mock fail` always fails.
- `agent:<type> [parameters...]` runs an agent. The type is `websearch` or
  `coder`, for example `agent:websearch rust release notes`. Agents are not
  implemented yet, so this action always fails.

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

# Show usage
cargo run -- --help
```

Atama runs the actions in the directory where you start it, so relative
paths such as `./output` are relative to that directory. The log shows each
action with a `$` prefix and each line of its output with a `|` prefix.

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
