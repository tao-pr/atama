//! Tests for the final checks of the agent loop, in
//! `src/agent_loop_openai_compat/checks.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;

use atama::agent_loop_openai_compat::checks::{
    CheckResult, CommandCheck, FinalCheck, MAX_CHECK_CHARS, Rejection, SchemaCheck, run_checks,
    tail,
};
use atama::schema::OutputSchema;

fn title_schema() -> OutputSchema {
    OutputSchema {
        name: "article".into(),
        schema: json!({
            "type": "object",
            "properties": {"title": {"type": "string"}},
            "required": ["title"],
        }),
    }
}

/// A check that records each reply that it gets, and gives `result`.
struct Fixed {
    result: CheckResult,
    seen: Rc<RefCell<Vec<String>>>,
}

impl FinalCheck for Fixed {
    fn check(&mut self, reply: &str) -> CheckResult {
        self.seen.borrow_mut().push(reply.to_string());
        self.result.clone()
    }

    fn rejection(&self) -> String {
        "fixed rejection".into()
    }

    fn failure(&self) -> String {
        "fixed failure".into()
    }
}

fn fixed(result: CheckResult) -> (Box<dyn FinalCheck>, Rc<RefCell<Vec<String>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let check = Fixed {
        result,
        seen: seen.clone(),
    };
    (Box::new(check), seen)
}

fn retry(message: &str, error: &str) -> CheckResult {
    CheckResult::Retry {
        message: message.into(),
        error: error.into(),
    }
}

#[test]
fn without_checks_the_output_is_the_reply() {
    assert_eq!(run_checks(&mut [], "reply"), Ok("reply".to_string()));
}

#[test]
fn each_check_gets_the_output_of_the_check_before_it() {
    let (first, first_seen) = fixed(CheckResult::Done("one".into()));
    let (second, second_seen) = fixed(CheckResult::Done("two".into()));
    let mut checks = vec![first, second];
    assert_eq!(run_checks(&mut checks, "reply"), Ok("two".to_string()));
    assert_eq!(*first_seen.borrow(), ["reply"]);
    assert_eq!(*second_seen.borrow(), ["one"]);
}

#[test]
fn the_first_failed_check_stops_the_list() {
    let (first, _) = fixed(CheckResult::Done("one".into()));
    let (second, _) = fixed(retry("try again", "bad"));
    let (third, third_seen) = fixed(CheckResult::Done("three".into()));
    let mut checks = vec![first, second, third];
    assert_eq!(
        run_checks(&mut checks, "reply"),
        Err(Rejection {
            index: 1,
            message: "try again".into(),
            error: "bad".into(),
        })
    );
    assert!(third_seen.borrow().is_empty());
    assert_eq!(checks[1].rejection(), "fixed rejection");
    assert_eq!(checks[1].failure(), "fixed failure");
}

#[test]
fn schema_check_gives_the_json_of_a_matching_reply() {
    let mut check = SchemaCheck::new(title_schema());
    assert_eq!(
        check.check("{\"title\": \"T\"}"),
        CheckResult::Done("{\"title\": \"T\"}".into())
    );
}

#[test]
fn schema_check_asks_the_model_again_for_a_reply_that_does_not_match() {
    let mut check = SchemaCheck::new(title_schema());
    let CheckResult::Retry { message, error } = check.check("{\"title\": 1}") else {
        panic!("the reply does not match the schema");
    };
    assert!(
        error.starts_with("the reply does not match the schema:\nat `/title`: "),
        "{error}"
    );
    assert_eq!(
        message,
        format!("Your reply is not valid: {error}\nReply again with only the JSON value.")
    );
    assert_eq!(
        check.rejection(),
        "reply does not match output schema `article`"
    );
    assert_eq!(
        check.failure(),
        "did not reply with JSON that matches output schema `article`"
    );
}

#[test]
fn command_check_passes_on_exit_status_0_and_keeps_the_reply() {
    let mut check = CommandCheck::new("true".into());
    assert_eq!(check.check("done"), CheckResult::Done("done".into()));
    assert!(check.ends_iteration());
    assert!(!SchemaCheck::new(title_schema()).ends_iteration());
}

#[test]
fn command_check_gives_the_status_stdout_and_stderr_of_a_failed_command() {
    let mut check = CommandCheck::new("echo '1 passed'; echo '2 failed' >&2; exit 3".into());
    let CheckResult::Retry { message, error } = check.check("done") else {
        panic!("the command fails");
    };
    assert_eq!(error, "exit status: 3\n1 passed\n2 failed");
    assert_eq!(
        message,
        "The task is not done yet, because the success check \
         `echo '1 passed'; echo '2 failed' >&2; exit 3` failed:\n\
         exit status: 3\n1 passed\n2 failed\n\
         Continue the work. When you are done, reply without tool calls."
    );
    assert_eq!(
        check.rejection(),
        "success check `echo '1 passed'; echo '2 failed' >&2; exit 3` failed"
    );
    assert_eq!(
        check.failure(),
        "did not pass its success check `echo '1 passed'; echo '2 failed' >&2; exit 3`"
    );
}

#[test]
fn command_check_keeps_the_end_of_a_long_output() {
    let mut check = CommandCheck::new(format!(
        "printf 'a%.0s' $(seq 1 {MAX_CHECK_CHARS}); echo; echo last line; exit 1"
    ));
    let CheckResult::Retry { error, .. } = check.check("done") else {
        panic!("the command fails");
    };
    assert!(
        error.starts_with("exit status: 1\n[cut: the output has "),
        "{error}"
    );
    assert!(error.ends_with("a\nlast line"), "{error}");
}

#[test]
fn tail_keeps_the_last_characters() {
    assert_eq!(tail("abc", 3), "abc");
    assert_eq!(
        tail("abcdé", 2),
        "[cut: the output has 5 characters, these are the last 2]\ndé"
    );
}

#[test]
fn command_check_fails_if_the_shell_cannot_start() {
    let mut check = CommandCheck::new("true".into()).with_shell("/no/such/shell");
    let CheckResult::Retry { error, .. } = check.check("done") else {
        panic!("the shell cannot start");
    };
    assert!(
        error.starts_with("cannot start /no/such/shell: "),
        "{error}"
    );
}
