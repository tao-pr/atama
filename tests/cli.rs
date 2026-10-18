//! Integration tests for the `atama` executable.
//!
//! Cargo builds the binary before running these tests and exposes its path
//! through the `CARGO_BIN_EXE_atama` environment variable.

use std::fs;
use std::path::PathBuf;
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
    // The fixture runs cargo commands, so only do a dry run.
    let output = run_atama(&["--dry-run", fixture]);

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

/// Creates an empty directory for one test and writes `job` to `job.md` in
/// it. Returns the directory and the path of the job file.
fn job_in_temp_dir(name: &str, job: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("atama-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let job = job.replace("{dir}", &dir.display().to_string());
    let path = dir.join("job.md");
    fs::write(&path, job).unwrap();
    (dir, path.display().to_string())
}

#[test]
fn runs_every_action_of_every_task() {
    let (dir, job) = job_in_temp_dir(
        "run",
        "# Job\n## Make\n- actions:\n  - bash mkdir -p {dir}/out\n  - bash touch {dir}/out/a.txt\n\
         ## Greet\n- actions: bash printf hello\n\
         ## List\n- actions: bash ls {dir}/out\n",
    );
    let output = run_atama(&[&job]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "stdout was: {stdout}");
    assert!(dir.join("out/a.txt").is_file());
    assert!(stdout.contains("[Greet] | hello"), "stdout was: {stdout}");
    assert!(stdout.contains("[List] | a.txt"), "stdout was: {stdout}");
    assert!(stdout.ends_with("job succeeded\n"), "stdout was: {stdout}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dry_run_runs_no_actions() {
    let (dir, job) = job_in_temp_dir("dry", "# Job\n## Make\n- actions: bash touch {dir}/a.txt\n");
    let output = run_atama(&["--dry-run", &job]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "stdout was: {stdout}");
    assert!(
        stdout.contains("[Make] $ bash touch"),
        "stdout was: {stdout}"
    );
    assert!(!dir.join("a.txt").exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fails_when_an_action_fails() {
    let (dir, job) = job_in_temp_dir("fail", "# Job\n## Broken\n- actions: mock fail\n");
    let output = run_atama(&[&job]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(!output.status.success(), "stdout was: {stdout}");
    assert!(
        stdout.contains("[Broken] failed: `mock fail`: mock failure"),
        "stdout was: {stdout}"
    );
    fs::remove_dir_all(dir).unwrap();
}
