//! Tests for `RunBudget`, the limits of one run of the agent loop, in
//! `src/agent_loop_openai_compat/budget.rs`.

use atama::agent_loop_openai_compat::budget::{RunBudget, StopReason};

#[test]
fn counts_steps_until_the_step_limit() {
    let mut budget = RunBudget::new(1, 2, None);
    assert_eq!(budget.steps(), 0);
    assert_eq!(budget.start_step(), Ok(1));
    assert_eq!(budget.start_step(), Ok(2));
    assert_eq!(budget.steps(), 2);
    assert_eq!(budget.start_step(), Err(StopReason::Steps { max: 2 }));
    assert_eq!(budget.steps(), 2);
}

#[test]
fn stops_when_the_run_used_its_token_budget() {
    let mut budget = RunBudget::new(1, 10, Some(100));
    assert_eq!(budget.token_usage(), Some((0, 100)));
    assert_eq!(budget.start_step(), Ok(1));
    assert_eq!(budget.add_tokens(Some(60)), Ok(()));
    assert_eq!(budget.start_step(), Ok(2));
    assert_eq!(budget.add_tokens(Some(40)), Ok(()));
    assert_eq!(budget.token_usage(), Some((100, 100)));
    assert_eq!(
        budget.start_step(),
        Err(StopReason::Tokens {
            used: 100,
            limit: 100,
            steps: 2
        })
    );
}

#[test]
fn the_step_limit_comes_before_the_token_budget() {
    let mut budget = RunBudget::new(1, 1, Some(10));
    budget.start_step().unwrap();
    budget.add_tokens(Some(50)).unwrap();
    assert_eq!(budget.start_step(), Err(StopReason::Steps { max: 1 }));
}

#[test]
fn a_reply_without_tokens_fails_only_with_a_token_budget() {
    let mut budget = RunBudget::new(1, 10, None);
    assert_eq!(budget.add_tokens(None), Ok(()));
    assert_eq!(budget.add_tokens(Some(5)), Ok(()));
    assert_eq!(budget.token_usage(), None);

    let mut budget = RunBudget::new(1, 10, Some(100));
    assert_eq!(
        budget.add_tokens(None),
        Err(StopReason::MissingTokens { limit: 100 })
    );
}

#[test]
fn counts_iterations_until_the_iteration_limit() {
    let mut budget = RunBudget::new(3, 10, None);
    assert_eq!(budget.iteration(), 1);
    assert_eq!(budget.next_iteration(), Ok(2));
    assert_eq!(budget.next_iteration(), Ok(3));
    assert_eq!(budget.iteration(), 3);
    assert_eq!(
        budget.next_iteration(),
        Err(StopReason::Iterations { max: 3 })
    );
}

#[test]
fn steps_count_for_the_whole_run_in_all_iterations() {
    let mut budget = RunBudget::new(2, 3, None);
    budget.start_step().unwrap();
    budget.start_step().unwrap();
    budget.next_iteration().unwrap();
    assert_eq!(budget.start_step(), Ok(3));
    assert_eq!(budget.start_step(), Err(StopReason::Steps { max: 3 }));
}

#[test]
fn each_stop_reason_has_an_error_that_names_the_agent() {
    let cases = [
        (
            StopReason::Steps { max: 2 },
            "`agent:coder` did not finish in 2 steps",
        ),
        (
            StopReason::Tokens {
                used: 110,
                limit: 100,
                steps: 2,
            },
            "`agent:coder` used 110 tokens in 2 steps, and its token budget is 100",
        ),
        (
            StopReason::MissingTokens { limit: 100 },
            "`agent:coder` has a token budget of 100, but the server did not send \
             `usage.total_tokens` in its response",
        ),
        (
            StopReason::Iterations { max: 3 },
            "`agent:coder` did not finish in 3 iterations",
        ),
    ];
    for (reason, expected) in cases {
        assert_eq!(reason.message("agent:coder"), expected);
    }
}
