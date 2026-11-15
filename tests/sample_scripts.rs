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

const WRITE_BY_KEY_SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/jobs/scripts/write_json_by_key.sh"
);

const JOIN_GGUF_SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/jobs/scripts/join_gguf_models.sh"
);

const CHECK_GGUF_MD_SCRIPT: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/jobs/scripts/check_gguf_md.sh");

/// Whether `jq` is installed. The scripts need it.
fn has_jq() -> bool {
    Command::new("jq")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Runs the split script with `dir` as its argument and `input` on stdin.
fn split(dir: &Path, input: &str) -> Output {
    run(SPLIT_SCRIPT, &[dir.as_os_str()], input)
}

/// Runs the write-by-key script with `dir` and `key` as its arguments and
/// `input` on stdin.
fn write_by_key(dir: &Path, key: &str, input: &str) -> Output {
    run(
        WRITE_BY_KEY_SCRIPT,
        &[dir.as_os_str(), std::ffi::OsStr::new(key)],
        input,
    )
}

/// Runs the write-by-key script with `dir`, `key` and `prefix` as its
/// arguments and `input` on stdin.
fn write_by_key_with_prefix(dir: &Path, key: &str, prefix: &str, input: &str) -> Output {
    run(
        WRITE_BY_KEY_SCRIPT,
        &[
            dir.as_os_str(),
            std::ffi::OsStr::new(key),
            std::ffi::OsStr::new(prefix),
        ],
        input,
    )
}

/// Runs `script` with `args` and `input` on stdin.
fn run(script: &str, args: &[&std::ffi::OsStr], input: &str) -> Output {
    let mut child = Command::new(script)
        .args(args)
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

#[test]
fn write_by_key_writes_each_object_to_the_file_that_its_key_names() {
    if !has_jq() {
        eprintln!("skipped: jq is not installed");
        return;
    }
    let dir = temp_dir("by-key");
    let out = dir.join("cities");
    // One object on one line, then one pretty object, then a blank line.
    let input = "{\"city\": \"berlin\", \"minTemp\": 3}\n{\n  \"city\": \"new_york-2\",\n  \"minTemp\": -1.5\n}\n\n";
    let output = write_by_key(&out, "city", input);
    assert!(output.status.success(), "{output:?}");
    let berlin = out.join("berlin.json");
    let new_york = out.join("new_york-2.json");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}\n{}\n", berlin.display(), new_york.display())
    );
    assert_eq!(read_json(&berlin), json!({"city": "berlin", "minTemp": 3}));
    assert_eq!(
        read_json(&new_york),
        json!({"city": "new_york-2", "minTemp": -1.5})
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn write_by_key_puts_the_prefix_before_each_file_name() {
    if !has_jq() {
        eprintln!("skipped: jq is not installed");
        return;
    }
    let dir = temp_dir("by-key-prefix");
    let input = "{\"provider\":\"openai\",\"models\":[]}\n";
    let output = write_by_key_with_prefix(&dir, "provider", "models-", input);
    assert!(output.status.success(), "{output:?}");
    let file = dir.join("models-openai.json");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}\n", file.display())
    );
    assert_eq!(
        read_json(&file),
        json!({"provider": "openai", "models": []})
    );

    let output = write_by_key_with_prefix(&dir, "provider", "../x-", input);
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "write_json_by_key.sh: `../x-` is not a file name prefix\n"
    );
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn write_by_key_writes_no_file_for_an_empty_input() {
    if !has_jq() {
        eprintln!("skipped: jq is not installed");
        return;
    }
    let dir = temp_dir("by-key-empty");
    let output = write_by_key(&dir, "city", "");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn write_by_key_fails_if_the_key_is_not_a_file_name() {
    if !has_jq() {
        eprintln!("skipped: jq is not installed");
        return;
    }
    let dir = temp_dir("by-key-bad");
    for input in [
        r#"{"city": "../berlin"}"#,
        r#"{"city": "new york"}"#,
        r#"{"city": 3}"#,
        r#"{"town": "berlin"}"#,
        "[1]",
    ] {
        let output = write_by_key(&dir, "city", input);
        assert!(!output.status.success(), "{input}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap().lines().last(),
            Some(
                format!(
                    "write_json_by_key.sh: `city` is not a file name in {}",
                    compact(input)
                )
                .as_str()
            ),
            "{input}"
        );
    }
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    let output = write_by_key(&dir, "city", "not json");
    assert!(!output.status.success(), "{output:?}");
    fs::remove_dir_all(dir).unwrap();
}

/// The JSON text on one line, as `jq -c` writes it.
fn compact(text: &str) -> String {
    serde_json::to_string(&serde_json::from_str::<Value>(text).unwrap()).unwrap()
}

/// Writes the `gguf.jsonl` and `models-*.json` files of the GGUF tests to
/// a new directory: `a` has two GGUF models, `b` has none, and `c` has one
/// but no entry in `models-x.json`.
fn gguf_dir(name: &str) -> PathBuf {
    let dir = temp_dir(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("gguf.jsonl"),
        concat!(
            r#"{"provider":"x","model":"a","gguf_models":[{"ollama_name":"hf.co/u/a-GGUF:Q4_K_M","url":"https://h.test/a"},{"ollama_name":"a:1b","url":"https://o.test/a"}]}"#,
            "\n",
            r#"{"provider":"x","model":"b","gguf_models":[]}"#,
            "\n",
            r#"{"provider":"y","model":"c","gguf_models":[{"ollama_name":"c:2b","url":"https://o.test/c"}]}"#,
            "\n",
        ),
    )
    .unwrap();
    fs::write(
        dir.join("models-x.json"),
        json!({"provider": "x", "models": [
            {"name": "b", "params": 1, "release_date": "2026-01-01",
             "short_description": "B.", "url": "https://m.test/b"},
            {"name": "a", "params": 7, "release_date": "2026-02-02",
             "short_description": "A.", "url": "https://m.test/a", "capabilities": []}
        ]})
        .to_string(),
    )
    .unwrap();
    dir
}

#[test]
fn join_gguf_keeps_the_models_with_gguf_and_adds_their_details() {
    if !has_jq() {
        eprintln!("skipped: jq is not installed");
        return;
    }
    let dir = gguf_dir("join-gguf");

    let output = run(JOIN_GGUF_SCRIPT, &[dir.as_os_str()], "");
    assert!(output.status.success(), "{output:?}");
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines,
        [
            json!({"provider": "x", "name": "a", "params": 7, "release_date": "2026-02-02",
                   "short_description": "A.", "url": "https://m.test/a",
                   "gguf_models": [
                       {"ollama_name": "hf.co/u/a-GGUF:Q4_K_M", "url": "https://h.test/a"},
                       {"ollama_name": "a:1b", "url": "https://o.test/a"}]}),
            json!({"provider": "y", "name": "c", "params": null, "release_date": null,
                   "short_description": null, "url": null,
                   "gguf_models": [{"ollama_name": "c:2b", "url": "https://o.test/c"}]}),
        ]
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn join_gguf_works_without_models_files() {
    if !has_jq() {
        eprintln!("skipped: jq is not installed");
        return;
    }
    let dir = gguf_dir("join-gguf-no-models");
    fs::remove_file(dir.join("models-x.json")).unwrap();

    let output = run(JOIN_GGUF_SCRIPT, &[dir.as_os_str()], "");
    assert!(output.status.success(), "{output:?}");
    let names: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap()["name"].clone())
        .collect();
    assert_eq!(names, [json!("a"), json!("c")]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_gguf_md_passes_only_if_each_name_is_in_the_file() {
    if !has_jq() {
        eprintln!("skipped: jq is not installed");
        return;
    }
    let dir = gguf_dir("check-gguf-md");
    let joined = run(JOIN_GGUF_SCRIPT, &[dir.as_os_str()], "");
    let jsonl = dir.join("gguf_models.jsonl");
    fs::write(&jsonl, joined.stdout).unwrap();
    let md = dir.join("gguf.md");
    let check = || {
        let output = run(
            CHECK_GGUF_MD_SCRIPT,
            &[jsonl.as_os_str(), md.as_os_str()],
            "",
        );
        (
            output.status.code(),
            String::from_utf8(output.stdout).unwrap(),
        )
    };

    assert_eq!(
        check(),
        (Some(1), format!("{} is missing or empty\n", md.display()))
    );

    fs::write(&md, "## a\n`hf.co/u/a-GGUF:Q4_K_M`\n## c\n").unwrap();
    assert_eq!(
        check(),
        (
            Some(1),
            format!("missing in {0}: a:1b\nmissing in {0}: c:2b\n", md.display())
        )
    );

    fs::write(&md, "## a\n`hf.co/u/a-GGUF:Q4_K_M`, `a:1b`\n## c\n`c:2b`\n").unwrap();
    assert_eq!(check(), (Some(0), String::new()));
    fs::remove_dir_all(dir).unwrap();
}
