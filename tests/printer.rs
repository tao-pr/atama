//! Tests for the job text printer.

use std::ffi::OsStr;

use atama::{
    TIMESTAMP_FORMAT, color_allowed, format_error, format_job, local_timestamp, parse_job,
};
use chrono::{Local, NaiveDateTime};

const EXAMPLE_JOB: &str = include_str!("fixtures/example_job.md");

#[test]
fn formats_job_without_color() {
    let job = parse_job(EXAMPLE_JOB).expect("example job should parse");

    let expected = "\
Job: Build and release
  - Build: bash cargo build --release (retry 2x if fail) (write output to build.log if succeed)
  - Test: bash cargo test (write stderr to test-errors.log if fail)
  - Lint: bash cargo clippy (die if fail)
  - Release: bash cargo publish (call API https://example.com/notify if succeed)
  - Cleanup:
";
    assert_eq!(format_job(&job, false), expected);
}

#[test]
fn joins_multiple_actions() {
    let job = parse_job("# Job\n## A\n- actions:\n  - bash npm ci\n  - mock\n")
        .expect("job should parse");

    assert_eq!(
        format_job(&job, false),
        "Job: Job\n  - A: bash npm ci; mock\n"
    );
}

#[test]
fn colors_task_titles_and_hooks() {
    let job =
        parse_job("# Job\n## A\n- actions: bash make\n- if_fail: die\n- if_succeed: restart\n")
            .expect("job should parse");

    let expected = "\x1b[1mJob: Job\x1b[0m\n  - \x1b[1;36mA:\x1b[0m bash make \
                    \x1b[31m(die if fail)\x1b[0m \x1b[32m(restart if succeed)\x1b[0m\n";
    assert_eq!(format_job(&job, true), expected);
}

#[test]
fn highlights_the_agent_type_of_agent_actions() {
    let job = parse_job(
        "# Job\n## Research\n- actions:\n  - agent:websearch model=m find it\n  \
         - agent:coder\n  - bash ls\n",
    )
    .expect("job should parse");

    let expected = "\x1b[1mJob: Job\x1b[0m\n  - \x1b[1;36mResearch:\x1b[0m \
                    \x1b[30;47magent:websearch\x1b[0m model=m find it; \
                    \x1b[30;47magent:coder\x1b[0m; bash ls\n";
    assert_eq!(format_job(&job, true), expected);

    let expected =
        "Job: Job\n  - Research: agent:websearch model=m find it; agent:coder; bash ls\n";
    assert_eq!(format_job(&job, false), expected);
}

#[test]
fn formats_error_with_and_without_color() {
    assert_eq!(format_error("bad job", false), "error: bad job");
    assert_eq!(
        format_error("bad job", true),
        "\x1b[31merror: bad job\x1b[0m"
    );
}

#[test]
fn colors_only_a_terminal_without_no_color() {
    assert!(color_allowed(true, None));
    assert!(
        color_allowed(true, Some(OsStr::new(""))),
        "empty counts as not set"
    );
    assert!(!color_allowed(true, Some(OsStr::new("1"))));
    assert!(!color_allowed(false, None));
}

#[test]
fn local_timestamp_is_the_current_local_time() {
    let before = Local::now().naive_local();
    let timestamp = local_timestamp();
    let after = Local::now().naive_local();

    assert_eq!(timestamp.len(), "2026-10-24 09:15:02".len(), "{timestamp}");
    let time = NaiveDateTime::parse_from_str(&timestamp, TIMESTAMP_FORMAT).expect(&timestamp);
    // The format has no fractions of a second.
    assert!(time <= after, "{timestamp}");
    assert!(time > before - chrono::Duration::seconds(1), "{timestamp}");
}
