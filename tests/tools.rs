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

mod approval {
    use std::cell::RefCell;
    use std::io::{Cursor, Write};
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::counter;
    use atama::tools::{
        Approval, Approver, Autonomy, Bash, DenyApprover, Limited, ListFiles, PromptApprover,
        ReadFile, Tool, WriteFile,
    };

    /// A writer that keeps what it gets, so that a test can read it.
    #[derive(Clone, Default)]
    struct Output(Arc<Mutex<Vec<u8>>>);

    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Output {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    /// A writer that always fails.
    struct Broken;

    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("broken"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn prompt(is_terminal: bool, answer: &str) -> (PromptApprover, Output) {
        let output = Output::default();
        let approver = PromptApprover::with_io(
            is_terminal,
            Box::new(Cursor::new(answer.to_string())),
            Box::new(output.clone()),
        );
        (approver, output)
    }

    /// An approver that answers with `allow` and counts its questions.
    struct Fixed {
        allow: bool,
        asked: Rc<RefCell<Vec<String>>>,
    }

    impl Approver for Fixed {
        fn approve(&mut self, tool: &str, _input: &serde_json::Value) -> Result<(), String> {
            self.asked.borrow_mut().push(tool.to_string());
            if self.allow {
                Ok(())
            } else {
                Err(format!("no to `{tool}`"))
            }
        }
    }

    /// The tools that an approver was asked about, in order.
    type Asked = Rc<RefCell<Vec<String>>>;

    fn fixed(allow: bool) -> (Rc<RefCell<dyn Approver>>, Asked) {
        let asked = Rc::new(RefCell::new(Vec::new()));
        let approver: Rc<RefCell<dyn Approver>> = Rc::new(RefCell::new(Fixed {
            allow,
            asked: asked.clone(),
        }));
        (approver, asked)
    }

    #[test]
    fn only_the_read_tools_are_read_only() {
        assert!(ReadFile.read_only());
        assert!(ListFiles.read_only());
        assert!(!WriteFile::default().read_only());
        assert!(!Bash::default().read_only());
        assert!(Limited::new(Box::new(ReadFile), None, None).read_only());
        let (tool, _) = counter();
        assert!(!tool.read_only());
    }

    #[test]
    fn reads_and_writes_the_levels_of_autonomy() {
        for (text, level) in [
            ("no_ask", Autonomy::NoAsk),
            ("ask_if_risky", Autonomy::AskIfRisky),
            ("always_ask", Autonomy::AlwaysAsk),
        ] {
            assert_eq!(text.parse::<Autonomy>().unwrap(), level);
            assert_eq!(level.to_string(), text);
        }
        assert_eq!(
            "sometimes".parse::<Autonomy>().unwrap_err(),
            "`autonomy=sometimes` is not `no_ask`, `ask_if_risky` or `always_ask`"
        );
    }

    #[test]
    fn each_level_asks_for_its_tools() {
        let read = ReadFile;
        let write = WriteFile::default();
        assert!(!Autonomy::NoAsk.must_ask(&read));
        assert!(!Autonomy::NoAsk.must_ask(&write));
        assert!(!Autonomy::AskIfRisky.must_ask(&read));
        assert!(Autonomy::AskIfRisky.must_ask(&write));
        assert!(Autonomy::AlwaysAsk.must_ask(&read));
        assert!(Autonomy::AlwaysAsk.must_ask(&write));
    }

    #[test]
    fn deny_approver_denies_each_call() {
        assert_eq!(
            DenyApprover.approve("bash", &json!({})).unwrap_err(),
            "the call of `bash` was denied, because `on_ask=deny` denies each call that waits \
             for approval"
        );
    }

    #[test]
    fn prompt_approver_asks_on_the_terminal() {
        for answer in ["y\n", "Yes\n", " YES "] {
            let (mut approver, output) = prompt(true, answer);
            approver
                .approve("local_bash", &json!({"command": "ls"}))
                .unwrap();
            assert_eq!(
                output.text(),
                "The agent wants to call `local_bash` with:\n{\n  \"command\": \"ls\"\n}\nallow? [y/N] "
            );
        }
        for answer in ["n\n", "\n", "", "maybe\n"] {
            let (mut approver, _) = prompt(true, answer);
            assert_eq!(
                approver.approve("bash", &json!({})).unwrap_err(),
                "the user denied the call of `bash`"
            );
        }
    }

    #[test]
    fn prompt_approver_denies_without_a_terminal() {
        let (mut approver, output) = prompt(false, "y\n");
        assert_eq!(
            approver.approve("bash", &json!({})).unwrap_err(),
            "the call of `bash` was denied, because no terminal can ask the user \
             (`on_ask=prompt`)"
        );
        assert_eq!(output.text(), "");
    }

    #[test]
    fn prompt_approver_denies_if_it_cannot_write() {
        let mut approver =
            PromptApprover::with_io(true, Box::new(Cursor::new("y\n")), Box::new(Broken));
        assert_eq!(
            approver.approve("bash", &json!({})).unwrap_err(),
            "the user denied the call of `bash`"
        );
    }

    #[test]
    fn prompt_approver_of_the_process_reads_stdin() {
        // The test process has no terminal on stdin in CI, and may have one
        // on a desktop. Only make sure that it can be made.
        let _ = PromptApprover::new();
        let _ = PromptApprover::default();
    }

    #[test]
    fn approval_asks_before_a_risky_call() {
        let (approver, asked) = fixed(true);
        let (tool, calls) = counter();
        let mut tool = Approval::new(tool, Autonomy::AskIfRisky, approver);
        assert_eq!(tool.name(), "counter");
        assert_eq!(tool.description(), "Counts its calls.");
        assert_eq!(tool.parameters(), json!({"type": "object"}));
        assert!(tool.available());
        assert!(!tool.read_only());
        tool.set_deadline_time(None);
        tool.set_protect(None);
        assert_eq!(tool.call(&json!({})).unwrap(), "ok");
        assert_eq!(*asked.borrow(), ["counter"]);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn a_denied_call_does_not_run() {
        let (approver, asked) = fixed(false);
        let (tool, calls) = counter();
        let mut tool = Approval::new(tool, Autonomy::AlwaysAsk, approver);
        assert_eq!(tool.call(&json!({})).unwrap_err(), "no to `counter`");
        assert_eq!(asked.borrow().len(), 1);
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn approval_does_not_ask_for_a_safe_call() {
        let (approver, asked) = fixed(false);
        let (tool, calls) = counter();
        let mut tool = Approval::new(tool, Autonomy::NoAsk, approver.clone());
        assert_eq!(tool.call(&json!({})).unwrap(), "ok");
        assert_eq!(calls.get(), 1);
        let mut read = Approval::new(Box::new(ReadFile), Autonomy::AskIfRisky, approver);
        assert!(read.read_only());
        assert!(read.call(&json!({})).is_err());
        assert!(asked.borrow().is_empty());
    }
}
