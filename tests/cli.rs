//! Integration tests for the `atama` executable.
//!
//! Cargo builds the binary before running these tests and exposes its path
//! through the `CARGO_BIN_EXE_atama` environment variable.

use std::process::{Command, Output};

/// Runs the `atama` binary with the given arguments and returns its output.
fn run_atama(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_atama"))
        .args(args)
        .output()
        .expect("failed to run atama binary")
}

#[test]
fn fails_without_job_config_file() {
    let output = run_atama(&[]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("<JOB_CONFIG_FILE>"), "stderr was: {stderr}");
}

#[test]
fn parses_job_config_file() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/example_job.md");
    let output = run_atama(&[fixture]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Build and release"), "stdout was: {stdout}");
}

#[test]
fn fails_on_missing_job_config_file() {
    let output = run_atama(&["does_not_exist.md"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot read"), "stderr was: {stderr}");
}

#[test]
fn prints_help() {
    let output = run_atama(&["--help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: atama"), "stdout was: {stdout}");
}
