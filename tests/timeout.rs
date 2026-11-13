//! Tests for the deadline of an agent run: Atama stops a command and its
//! child processes when the time limit of the run is used.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use atama::AgentToolLoop;
use atama::actor::{AgentType, run_bash, run_process};
use atama::agent_loop_openai_compat::checks::{CheckResult, CommandCheck, FinalCheck};
use atama::agent_loop_openai_compat::judge::Judge;
use atama::agent_loop_openai_compat::tools::{Bash, Limited};
use atama::agent_loop_openai_compat::{ChatClient, Message, Reply, Tool, ToolCall};

fn in_ms(ms: u64) -> Option<Instant> {
    Some(Instant::now() + Duration::from_millis(ms))
}

/// A fresh path `<target tmp>/<name>`.
fn fresh_path(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// Whether the process `pid` still runs, after up to 3 seconds of waiting
/// for it to stop.
fn still_runs(pid: &str) -> bool {
    for _ in 0..300 {
        let alive = Command::new("kill")
            .args(["-0", pid])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success();
        if !alive {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    true
}

#[test]
fn a_command_that_ends_before_the_deadline_gives_its_output() {
    assert_eq!(
        run_bash("echo hi", &[], "", in_ms(10_000)),
        Ok("hi\n".into())
    );
    assert_eq!(run_bash("echo hi", &[], "", None), Ok("hi\n".into()));
    let err = run_bash("echo bad >&2; exit 2", &[], "", in_ms(10_000)).unwrap_err();
    assert_eq!(
        err,
        "`echo bad >&2; exit 2` failed with exit status: 2: bad"
    );
}

#[test]
fn stops_a_command_at_the_deadline() {
    let start = Instant::now();
    let err = run_bash("sleep 10", &[], "", in_ms(300)).unwrap_err();
    assert_eq!(
        err,
        "`sleep 10` stopped after 0s, because the time limit of the run is used"
    );
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn stops_a_command_at_once_if_the_deadline_is_past() {
    let start = Instant::now();
    let past = Some(Instant::now());
    assert!(run_bash("sleep 10", &[], "", past).is_err());
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn stops_the_child_processes_of_the_command_too() {
    let pid_file = fresh_path("timeout_child.pid");
    let script = format!("sleep 30 & echo $! > '{}'; wait", pid_file.display());
    assert!(run_bash(&script, &[], "", in_ms(500)).is_err());
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    assert!(
        !still_runs(pid.trim()),
        "the child process {pid} still runs"
    );
}

#[test]
fn run_process_gives_the_status_and_both_outputs() {
    let mut command = Command::new("bash");
    command.args(["-c", "cat; echo err >&2; exit 4"]);
    let output = run_process(command, "bash", "cmd", "input", in_ms(10_000)).unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(output.stdout, b"input");
    assert_eq!(output.stderr, b"err\n");

    let err = run_process(Command::new("/no/such/program"), "prog", "cmd", "", None).unwrap_err();
    assert!(err.starts_with("cannot start prog: "), "{err}");
}

#[test]
fn the_bash_tool_stops_its_command_at_the_deadline() {
    let mut tool = Limited::new(Box::new(Bash::default()), None, None);
    tool.set_deadline_time(in_ms(300));
    let start = Instant::now();
    let err = tool.call(&json!({"command": "sleep 10"})).unwrap_err();
    assert!(err.starts_with("`sleep 10` stopped after "), "{err}");
    assert!(start.elapsed() < Duration::from_secs(3));

    tool.set_deadline_time(None);
    assert_eq!(tool.call(&json!({"command": "echo ok"})), Ok("ok\n".into()));
}

#[test]
fn the_success_check_stops_at_the_deadline() {
    let mut check = CommandCheck::new("sleep 10".into()).with_deadline_time(in_ms(300));
    let mut client = ScriptedClient {
        replies: VecDeque::new(),
        conversations: Vec::new(),
    };
    let mut judge = Judge::new(&mut client);
    let CheckResult::Retry { error, .. } = check.check("done", &mut judge, &[]) else {
        panic!("the check stops at the deadline");
    };
    assert_eq!(
        error,
        "`sleep 10` stopped after 0s, because the time limit of the run is used"
    );
}

/// A client that gives scripted replies.
struct ScriptedClient {
    replies: VecDeque<Reply>,
    conversations: Vec<Vec<Message>>,
}

impl ChatClient for ScriptedClient {
    fn chat(&mut self, messages: &[Message], _: &[Box<dyn Tool>]) -> Result<Reply, String> {
        self.conversations.push(messages.to_vec());
        Ok(self.replies.pop_front().expect("no more replies"))
    }
}

#[test]
fn the_time_limit_of_a_run_stops_a_long_bash_call() {
    let mut client = ScriptedClient {
        replies: VecDeque::from([Reply {
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "bash".into(),
                arguments: json!({"command": "sleep 10"}).to_string(),
            }],
            ..Default::default()
        }]),
        conversations: Vec::new(),
    };
    let start = Instant::now();
    let err = AgentToolLoop::new(AgentType::Autonomous)
        .max_steps(5)
        .max_time(Some(Duration::from_secs(1)))
        .run_with(&mut client, "task", "")
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:autonomous` did not finish in its time limit of 1s"
    );
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(client.conversations.len(), 1);
}
