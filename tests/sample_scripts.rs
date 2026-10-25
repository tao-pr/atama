//! Tests for the helper scripts of the sample jobs, in `jobs/scripts/`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const SPLIT_SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/jobs/scripts/split_json_array.sh"
);

/// Whether `jq` is installed. The scripts need it.
fn has_jq() -> bool {
    Command::new("jq")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Runs the split script with `dir` as its argument and `input` on stdin.
fn split(dir: &Path, input: &str) -> Output {
    let mut child = Command::new(SPLIT_SCRIPT)
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cannot start the script");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atama-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn split_writes_each_element_to_its_own_file_and_prints_the_paths() {
    if !has_jq() {
        eprintln!("skipped: jq is not installed");
        return;
    }
    let dir = temp_dir("split");
    let out = dir.join("results");
    let input = r#"[{"title": "A", "url": "https://a.test"}, {"title": "B"}, 3]"#;

    let output = split(&out, input);
    assert!(output.status.success(), "{output:?}");
    // The output is the path of each file, one per line, in order.
    let paths: Vec<PathBuf> = (1..=3)
        .map(|n| out.join(format!("result-{n}.json")))
        .collect();
    let expected: String = paths
        .iter()
        .map(|path| format!("{}\n", path.display()))
        .collect();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    let mut files: Vec<String> = fs::read_dir(&out)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(files, ["result-1.json", "result-2.json", "result-3.json"]);
    assert_eq!(
        read_json(&out.join("result-1.json")),
        json!({"title": "A", "url": "https://a.test"})
    );
    assert_eq!(read_json(&out.join("result-2.json")), json!({"title": "B"}));
    assert_eq!(read_json(&out.join("result-3.json")), json!(3));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn split_writes_no_file_for_an_empty_array() {
    if !has_jq() {
        eprintln!("skipped: jq is not installed");
        return;
    }
    let dir = temp_dir("split-empty");
    let output = split(&dir, "[]");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn split_fails_if_the_input_is_not_an_array() {
    if !has_jq() {
        eprintln!("skipped: jq is not installed");
        return;
    }
    let dir = temp_dir("split-object");
    for input in [r#"{"results": []}"#, "not json"] {
        let output = split(&dir, input);
        assert!(!output.status.success(), "{input}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap().lines().last(),
            Some("split_json_array.sh: the input is not a JSON array"),
            "{input}"
        );
    }
    assert!(!dir.exists());
}
