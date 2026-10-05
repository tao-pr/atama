//! Tests for the job text printer.

use atama::{format_job, parse_job};

const EXAMPLE_JOB: &str = include_str!("fixtures/example_job.md");

#[test]
fn formats_job_without_color() {
    let job = parse_job(EXAMPLE_JOB).expect("example job should parse");

    let expected = "\
Job: Build and release
  - Build: cargo build --release (retry 2x if fail) (write output to build.log if succeed)
  - Test: cargo test (write stderr to test-errors.log if fail)
  - Lint: cargo clippy (die if fail)
  - Release: cargo publish (call API https://example.com/notify if succeed)
  - Cleanup:
";
    assert_eq!(format_job(&job, false), expected);
}

#[test]
fn joins_multiple_actions() {
    let job =
        parse_job("# Job\n## A\n- actions:\n  - npm ci\n  - npm test\n").expect("job should parse");

    assert_eq!(
        format_job(&job, false),
        "Job: Job\n  - A: npm ci; npm test\n"
    );
}

#[test]
fn colors_task_titles_and_hooks() {
    let job = parse_job("# Job\n## A\n- actions: make\n- if_fail: die\n- if_succeed: restart\n")
        .expect("job should parse");

    let expected = "\x1b[1mJob: Job\x1b[0m\n  - \x1b[1;36mA:\x1b[0m make \
                    \x1b[31m(die if fail)\x1b[0m \x1b[32m(restart if succeed)\x1b[0m\n";
    assert_eq!(format_job(&job, true), expected);
}
