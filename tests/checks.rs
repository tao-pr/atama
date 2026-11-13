//! Tests for the final checks of the agent loop, in
//! `src/agent_loop_openai_compat/checks.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;

use atama::agent_loop_openai_compat::checks::{
    CheckResult, FinalCheck, Rejection, SchemaCheck, run_checks,
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
