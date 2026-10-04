# Build and release

Builds the project, runs the tests and publishes a release.

## Build

Compile the release binary.

- actions:
  - cargo build --release
- if_fail: retry=2
- if_succeed: write_file(build.log)

## Test

- begin_if: prev task succeed
- actions:
  - cargo test
- if_fail: write_error_file(test-errors.log)

## Lint

Runs in parallel with the tests.

- begin_if: task(Test) begin
- actions:
  - cargo clippy
- if_fail: die

## Release

- begin_if: task(Test) succeed
- actions:
  - cargo publish
- if_succeed: call_api(https://example.com/notify)

## Cleanup

- begin_if: prev task succeed or fail
