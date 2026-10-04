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
Task ids must be unique. See `tests/fixtures/example_job.md` for a full example.

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

# Or run a built binary directly
./target/release/atama job_config_file.md

# Show usage
cargo run -- --help
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
