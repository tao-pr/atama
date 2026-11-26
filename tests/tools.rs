//! Tests of the shared tool layer in `src/tools.rs`.

use std::cell::Cell;
use std::rc::Rc;

use serde_json::{Value, json};

use atama::tools::{
    Bash, DEFAULT_MAX_CALLS, DEFAULT_MAX_FAILURES, Limited, ListFiles, MAX_RESULT_CHARS, ReadFile,
    TOOL_NAMES, Tool, WriteFile, tool_by_name, truncate,
};

/// A tool that fails when its argument `fail` is true, and counts its calls.
struct Counter {
    calls: Rc<Cell<u32>>,
}

impl Tool for Counter {
    fn name(&self) -> &str {
        "counter"
    }

    fn description(&self) -> &str {
        "Counts its calls."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }

    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        self.calls.set(self.calls.get() + 1);
        if arguments["fail"] == json!(true) {
            Err("failed".into())
        } else {
            Ok("ok".into())
        }
    }
}

fn counter() -> (Box<dyn Tool>, Rc<Cell<u32>>) {
    let calls = Rc::new(Cell::new(0));
    (
        Box::new(Counter {
            calls: calls.clone(),
        }),
        calls,
    )
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("atama-tools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn registry_makes_each_tool_by_its_name() {
    for name in TOOL_NAMES {
        let tool = tool_by_name(name).unwrap();
        assert_eq!(tool.name(), name);
        assert!(!tool.description().is_empty());
        assert_eq!(tool.parameters()["type"], "object");
        assert!(tool.available());
    }
    assert!(tool_by_name("unknown").is_none());
}

#[test]
fn old_paths_name_the_same_items() {
    use atama::agent_loop_openai_compat::tools as old;

    // The trait of the OpenAI loop is the shared trait.
    let tool: Box<dyn atama::agent_loop_openai_compat::Tool> = Box::new(Bash::default());
    let _: Box<dyn Tool> = tool;
    let _: Bash = old::Bash::default();
    let _: ReadFile = old::ReadFile;
    let _: WriteFile = old::WriteFile::default();
    let _: ListFiles = old::ListFiles;
    let _: Limited = old::Limited::new(Box::new(ReadFile), None, None);
    assert_eq!(old::TOOL_NAMES, TOOL_NAMES);
    assert_eq!(old::MAX_RESULT_CHARS, MAX_RESULT_CHARS);
    assert_eq!(old::DEFAULT_MAX_CALLS, DEFAULT_MAX_CALLS);
    assert_eq!(old::DEFAULT_MAX_FAILURES, DEFAULT_MAX_FAILURES);
    assert_eq!(old::tool_by_name("bash").unwrap().name(), "bash");
    assert_eq!(old::truncate("x".into()), "x");
}

#[test]
fn limited_stops_at_its_call_limit() {
    let (tool, calls) = counter();
    let mut tool = Limited::new(tool, Some(2), None);
    assert_eq!(tool.name(), "counter");
    assert_eq!(tool.description(), "Counts its calls.");
    assert_eq!(tool.parameters(), json!({"type": "object"}));
    assert_eq!(tool.call(&json!({})).unwrap(), "ok");
    assert_eq!(tool.call(&json!({})).unwrap(), "ok");
    assert!(!tool.available());
    assert_eq!(
        tool.call(&json!({})).unwrap_err(),
        "tool `counter` used its limit of 2 calls"
    );
    assert_eq!(calls.get(), 2);
}

#[test]
fn limited_stops_at_its_failure_limit() {
    let (tool, calls) = counter();
    let mut tool = Limited::new(tool, None, Some(1));
    assert_eq!(tool.call(&json!({"fail": true})).unwrap_err(), "failed");
    assert!(!tool.available());
    assert_eq!(
        tool.call(&json!({})).unwrap_err(),
        "tool `counter` failed 1 times, which is its limit"
    );
    assert_eq!(calls.get(), 1);
}

#[test]
fn limited_without_limits_always_runs() {
    let (tool, calls) = counter();
    let mut tool = Limited::new(tool, None, None);
    for _ in 0..3 {
        let _ = tool.call(&json!({"fail": true}));
    }
    assert!(tool.available());
    assert_eq!(calls.get(), 3);
}

#[test]
fn truncate_cuts_long_results() {
    assert_eq!(truncate("short".into()), "short");
    let exact = "é".repeat(MAX_RESULT_CHARS);
    assert_eq!(truncate(exact.clone()), exact);
    assert_eq!(
        truncate(format!("{exact}more")),
        format!("{exact}\n[cut: the result has more than {MAX_RESULT_CHARS} characters]")
    );
}

#[test]
fn file_tools_write_read_and_list() {
    let dir = temp_dir("files");
    let path = dir.join("sub/a.txt");
    let path = path.to_str().unwrap();
    assert_eq!(
        WriteFile::default()
            .call(&json!({"path": path, "content": "héllo"}))
            .unwrap(),
        format!("wrote 5 characters to `{path}`")
    );
    assert_eq!(ReadFile.call(&json!({"path": path})).unwrap(), "héllo");
    let sub = dir.join("sub");
    let sub = sub.to_str().unwrap();
    assert_eq!(ListFiles.call(&json!({"dir": sub})).unwrap().trim(), path);
    assert_eq!(
        ListFiles
            .call(&json!({"dir": sub, "pattern": "*.md"}))
            .unwrap()
            .trim(),
        ""
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn tools_fail_on_a_missing_argument() {
    assert_eq!(
        ReadFile.call(&json!({})).unwrap_err(),
        "missing string argument `path`"
    );
    assert_eq!(
        WriteFile::default()
            .call(&json!({"path": "x"}))
            .unwrap_err(),
        "missing string argument `content`"
    );
    assert_eq!(
        ListFiles
            .call(&json!({"dir": ".", "pattern": 1}))
            .unwrap_err(),
        "missing string argument `pattern`"
    );
    assert_eq!(
        Bash::default().call(&json!({})).unwrap_err(),
        "missing string argument `command`"
    );
}

#[test]
fn bash_returns_stdout() {
    assert_eq!(
        Bash::default()
            .call(&json!({"command": "echo hi"}))
            .unwrap()
            .trim(),
        "hi"
    );
}
