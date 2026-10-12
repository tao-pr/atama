//! Tests for reading values from a passer.

use std::fs;
use std::path::PathBuf;

use atama::Passer;

/// Writes `contents` to a file in Cargo's temporary directory for tests.
fn temp_file(name: &str, contents: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    fs::write(&path, contents).expect("cannot write temp file");
    path
}

#[test]
fn returns_data_as_is() {
    let passer = Passer::Data("  secret value\n".into());

    assert_eq!(passer.get(), Ok("  secret value\n".into()));
}

#[test]
fn reads_file_without_trailing_newlines() {
    let path = temp_file("passer_token.txt", "token-123\n\n");
    let passer = Passer::FromFile(path.display().to_string());

    assert_eq!(passer.get(), Ok("token-123".into()));
}

#[test]
fn fails_on_missing_file() {
    let passer = Passer::FromFile("does/not/exist.txt".into());

    let err = passer.get().unwrap_err();
    assert!(err.contains("does/not/exist.txt"), "error was: {err}");
}

#[test]
fn reads_environment_variable() {
    // Cargo sets this variable when it runs tests.
    let passer = Passer::FromEnvVar("CARGO_MANIFEST_DIR".into());

    assert_eq!(passer.get(), Ok(env!("CARGO_MANIFEST_DIR").into()));
}

#[test]
fn fails_on_unset_environment_variable() {
    let passer = Passer::FromEnvVar("ATAMA_TEST_VARIABLE_THAT_IS_NOT_SET".into());

    let err = passer.get().unwrap_err();
    assert!(
        err.contains("ATAMA_TEST_VARIABLE_THAT_IS_NOT_SET"),
        "error was: {err}"
    );
}

#[test]
fn reads_command_stdout_without_trailing_newline() {
    let passer = Passer::FromStdoutOfCommand("echo hello world".into());

    assert_eq!(passer.get(), Ok("hello world".into()));
}

#[test]
fn fails_when_command_fails() {
    let passer = Passer::FromStdoutOfCommand("echo oops >&2; exit 3".into());

    let err = passer.get().unwrap_err();
    assert!(err.contains("oops"), "error was: {err}");
}
