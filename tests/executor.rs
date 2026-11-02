//! Tests for the sequential job executor.

use std::collections::{HashMap, VecDeque};

use atama::{
    ActionRunner, Actor, ActorRunner, DryRunner, Executor, JobOutcome, JobReport, TaskStatus,
    parse_job,
};

/// Runner whose actions fail on cue, recording every action it runs.
///
/// The jobs in these tests use `bash <name>` actions, and the runner uses
/// `<name>` as the action name. It never runs bash.
///
/// A successful action outputs its input followed by its name and a space,
/// so tests can see how output passes from action to action.
#[derive(Default)]
struct ScriptedRunner {
    /// Per action name, the result of each call in order. Once a queue is
    /// empty, that action succeeds.
    results: HashMap<String, VecDeque<bool>>,
    calls: Vec<String>,
    /// The input of each call, in order.
    inputs: Vec<String>,
}

impl ScriptedRunner {
    /// Makes the action named `name` fail the next `times` calls.
    fn fail(mut self, name: &str, times: usize) -> Self {
        self.results
            .entry(name.to_string())
            .or_default()
            .extend(std::iter::repeat_n(false, times));
        self
    }

    fn calls_to(&self, name: &str) -> usize {
        self.calls.iter().filter(|call| *call == name).count()
    }
}

impl ActionRunner for ScriptedRunner {
    fn run(&mut self, action: &Actor, input: &str) -> Result<String, String> {
        let name = match action {
            Actor::BashScript { cmd, .. } => cmd.clone(),
            other => other.to_string(),
        };
        self.calls.push(name.clone());
        self.inputs.push(input.to_string());
        let succeeded = self
            .results
            .get_mut(&name)
            .and_then(VecDeque::pop_front)
            .unwrap_or(true);
        if succeeded {
            Ok(format!("{input}{name} "))
        } else {
            Err("scripted failure".to_string())
        }
    }
}

/// Parses `input`, runs it with `runner`, and returns the report and log.
fn run<R: ActionRunner>(input: &str, runner: &mut R) -> (JobReport, String) {
    let job = parse_job(input).expect("job should parse");
    let mut log = Vec::new();
    let report = Executor::new(&job, runner, &mut log)
        .run()
        .expect("writing to a Vec cannot fail");
    (report, String::from_utf8(log).expect("log is UTF-8"))
}

fn statuses(report: &JobReport) -> Vec<TaskStatus> {
    report.tasks.iter().map(|task| task.status).collect()
}

#[test]
fn runs_tasks_in_order_and_logs_actions() {
    let job = parse_job(include_str!("fixtures/example_job.md")).expect("job should parse");
    let mut log = Vec::new();
    let report = Executor::new(&job, DryRunner, &mut log).run().unwrap();
    let log = String::from_utf8(log).unwrap();

    assert!(report.succeeded());
    assert_eq!(report.outcome, JobOutcome::Completed);
    assert!(
        log.contains("[Build] $ bash cargo build --release"),
        "log was:\n{log}"
    );
    assert!(
        log.contains("[Build] hook: write output to build.log (not implemented yet)"),
        "log was:\n{log}"
    );
    let build = log.find("[Build] start").unwrap();
    let test = log.find("[Test] start").unwrap();
    assert!(build < test, "log was:\n{log}");
    assert!(log.ends_with("job succeeded\n"), "log was:\n{log}");
}

#[test]
fn does_not_retry_when_first_run_succeeds() {
    let mut runner = ScriptedRunner::default();
    let (report, log) = run(
        "# Job\n## A\n- actions: bash flaky\n- if_fail: retry=3\n",
        &mut runner,
    );

    assert_eq!(runner.calls_to("flaky"), 1);
    assert_eq!(report.tasks[0].status, TaskStatus::Succeeded);
    assert_eq!(report.tasks[0].attempts, 1);
    assert!(!log.contains("retry"), "log was:\n{log}");
    assert!(report.succeeded());
}

#[test]
fn retries_up_to_n_times_then_fails() {
    let mut runner = ScriptedRunner::default().fail("flaky", 10);
    let (report, log) = run(
        "# Job\n## A\n- actions: bash flaky\n- if_fail: retry=2\n",
        &mut runner,
    );

    assert_eq!(runner.calls_to("flaky"), 3);
    assert_eq!(report.tasks[0].status, TaskStatus::Failed);
    assert_eq!(report.tasks[0].attempts, 3);
    assert!(log.contains("[A] retry 2 of 2"), "log was:\n{log}");
    assert!(!log.contains("retry 3"), "log was:\n{log}");
    assert!(!report.succeeded());
}

#[test]
fn stops_retrying_once_task_succeeds() {
    let mut runner = ScriptedRunner::default().fail("flaky", 1);
    let (report, log) = run(
        "# Job\n## A\n- actions: bash flaky\n- if_fail: retry=5\n- if_succeed: write_file(out.log)\n",
        &mut runner,
    );

    assert_eq!(runner.calls_to("flaky"), 2);
    assert_eq!(report.tasks[0].status, TaskStatus::Succeeded);
    assert_eq!(report.tasks[0].attempts, 2);
    assert!(
        log.contains("[A] hook: write output to out.log"),
        "log was:\n{log}"
    );
    assert!(report.succeeded());
}

#[test]
fn runs_once_without_retry_hook() {
    let mut runner = ScriptedRunner::default().fail("flaky", 10);
    let (report, _) = run("# Job\n## A\n- actions: bash flaky\n", &mut runner);

    assert_eq!(runner.calls_to("flaky"), 1);
    assert_eq!(report.tasks[0].attempts, 1);
}

#[test]
fn stops_task_at_first_failing_action() {
    let mut runner = ScriptedRunner::default().fail("first", 1);
    let (_, log) = run(
        "# Job\n## A\n- actions:\n  - bash first\n  - bash second\n",
        &mut runner,
    );

    assert_eq!(runner.calls, ["first"]);
    assert!(
        log.contains("[A] failed: `bash first`: scripted failure"),
        "log was:\n{log}"
    );
}

#[test]
fn die_hook_stops_the_job() {
    let mut runner = ScriptedRunner::default().fail("broken", 1);
    let (report, _) = run(
        "# Job\n## A\n- actions: bash broken\n- if_fail: die\n\
         ## B\n- begin_if: prev task fail\n- actions: bash after\n",
        &mut runner,
    );

    assert_eq!(report.outcome, JobOutcome::Died);
    assert_eq!(statuses(&report), [TaskStatus::Failed, TaskStatus::NotRun]);
    assert_eq!(runner.calls_to("after"), 0);
}

#[test]
fn restart_hook_reruns_the_job_from_the_first_task() {
    let mut runner = ScriptedRunner::default().fail("flaky", 1);
    let (report, _) = run(
        "# Job\n## A\n- actions: bash setup\n## B\n- actions: bash flaky\n- if_fail: restart\n",
        &mut runner,
    );

    assert_eq!(runner.calls, ["setup", "flaky", "setup", "flaky"]);
    assert_eq!(report.restarts, 1);
    assert!(report.succeeded());
}

#[test]
fn restart_hook_gives_up_after_max_restarts() {
    let job = parse_job("# Job\n## A\n- actions: bash broken\n- if_fail: restart\n").unwrap();
    let mut runner = ScriptedRunner::default().fail("broken", 100);
    let report = Executor::new(&job, &mut runner, std::io::sink())
        .max_restarts(2)
        .run()
        .unwrap();

    assert_eq!(report.outcome, JobOutcome::RestartLimitReached);
    assert_eq!(report.restarts, 2);
    assert_eq!(runner.calls_to("broken"), 3);
}

#[test]
fn skips_tasks_whose_begin_if_is_not_met() {
    let mut runner = ScriptedRunner::default();
    let (report, log) = run(
        "# Job\n## Deploy\n- actions: bash deploy\n\
         ## Rollback\n- begin_if: prev task fail\n- actions: bash rollback\n\
         ## Report\n- begin_if: task(Deploy) succeed or fail\n- actions: bash report\n\
         ## Notify\n- actions: bash notify\n",
        &mut runner,
    );

    assert_eq!(
        statuses(&report),
        [
            TaskStatus::Succeeded,
            TaskStatus::Skipped,
            TaskStatus::Succeeded,
            TaskStatus::Succeeded,
        ]
    );
    assert_eq!(runner.calls, ["deploy", "report", "notify"]);
    assert!(
        log.contains("[Rollback] skipped (begin_if not met)"),
        "log was:\n{log}"
    );
}

#[test]
fn runs_failure_branch_when_task_fails() {
    let mut runner = ScriptedRunner::default().fail("deploy", 1);
    let (report, _) = run(
        "# Job\n## Deploy\n- actions: bash deploy\n\
         ## Rollback\n- begin_if: prev task fail\n- actions: bash rollback\n\
         ## Next\n- actions: bash next\n",
        &mut runner,
    );

    assert_eq!(runner.calls, ["deploy", "rollback", "next"]);
    assert_eq!(report.tasks[1].status, TaskStatus::Succeeded);
    assert!(!report.succeeded(), "Deploy failed, so the job failed");
}

#[test]
fn passes_output_from_action_to_action_and_task_to_task() {
    let mut runner = ScriptedRunner::default();
    run(
        "# Job\n## A\n- actions:\n  - bash a1\n  - bash a2\n## B\n- actions: bash b1\n",
        &mut runner,
    );

    assert_eq!(runner.calls, ["a1", "a2", "b1"]);
    assert_eq!(runner.inputs, ["", "a1 ", "a1 a2 "]);
}

#[test]
fn passes_output_of_last_successful_task() {
    let mut runner = ScriptedRunner::default().fail("broken", 1);
    run(
        "# Job\n## A\n- actions: bash a1\n\
         ## B\n- actions: bash broken\n\
         ## C\n- begin_if: prev task fail\n- actions: bash c1\n",
        &mut runner,
    );

    assert_eq!(runner.calls, ["a1", "broken", "c1"]);
    assert_eq!(runner.inputs, ["", "a1 ", "a1 "]);
}

#[test]
fn retry_gets_the_same_input() {
    let mut runner = ScriptedRunner::default().fail("flaky", 1);
    run(
        "# Job\n## A\n- actions: bash a1\n## B\n- actions: bash flaky\n- if_fail: retry=1\n",
        &mut runner,
    );

    assert_eq!(runner.inputs, ["", "a1 ", "a1 "]);
}

#[test]
fn mock_actions_from_a_job_file_pass_input_and_fail() {
    let job = parse_job(
        "# Job\n## A\n- actions:\n  - bash printf hello\n  - mock\n\
         ## B\n- actions:\n  - mock\n  - mock fail\n  - bash never-runs\n- if_fail: retry=1\n\
         ## C\n- begin_if: prev task fail\n- actions: bash grep -qx hello\n",
    )
    .expect("job should parse");
    let mut log = Vec::new();
    let report = Executor::new(&job, ActorRunner::default(), &mut log)
        .run()
        .unwrap();
    let log = String::from_utf8(log).unwrap();

    // C succeeds only if its input is `hello`: the output of A, passed
    // through `mock`.
    assert_eq!(
        statuses(&report),
        [
            TaskStatus::Succeeded,
            TaskStatus::Failed,
            TaskStatus::Succeeded
        ]
    );
    assert_eq!(report.tasks[1].attempts, 2);
    assert!(
        log.contains("[B] failed: `mock fail`: mock failure"),
        "log was:\n{log}"
    );
    assert!(!log.contains("never-runs"), "log was:\n{log}");
}

#[test]
fn logs_each_line_of_action_output() {
    let job = parse_job("# Job\n## A\n- actions:\n  - bash printf one\\ntwo\n  - bash true\n")
        .expect("job should parse");
    let mut log = Vec::new();
    Executor::new(&job, ActorRunner::default(), &mut log)
        .run()
        .unwrap();
    let log = String::from_utf8(log).unwrap();

    assert!(
        log.contains(
            "[A] $ bash printf one\\ntwo\n[A] | one\n[A] | two\n[A] $ bash true\n[A] succeeded\n"
        ),
        "log was:\n{log}"
    );
}

#[test]
fn starts_tasks_on_another_tasks_begin_fail_or_succeed() {
    let mut runner = ScriptedRunner::default().fail("broken", 1);
    let (report, _) = run(
        "# Job\n## A\n- actions: bash broken\n\
         ## Begun\n- begin_if: task(A) begin\n- actions: bash begun\n\
         ## Failed\n- begin_if: task(A) fail\n- actions: bash failed\n\
         ## Succeeded\n- begin_if: task(A) succeed\n- actions: bash succeeded\n\
         ## After skip\n- begin_if: task(Succeeded) begin\n- actions: bash after\n",
        &mut runner,
    );

    assert_eq!(runner.calls, ["broken", "begun", "failed"]);
    assert_eq!(
        statuses(&report),
        [
            TaskStatus::Failed,
            TaskStatus::Succeeded,
            TaskStatus::Succeeded,
            TaskStatus::Skipped,
            TaskStatus::Skipped,
        ]
    );
}

#[test]
fn logs_hooks_that_are_not_implemented_yet() {
    let mut runner = ScriptedRunner::default().fail("broken", 1);
    let (_, log) = run(
        "# Job\n## A\n- actions: bash ok\n- if_succeed: call_api(https://x.test/done)\n\
         ## B\n- actions: bash broken\n- if_fail: write_error_file(err.log)\n",
        &mut runner,
    );

    assert!(
        log.contains("[A] hook: call API https://x.test/done (not implemented yet)"),
        "log was:\n{log}"
    );
    assert!(
        log.contains("[B] hook: write stderr to err.log (not implemented yet)"),
        "log was:\n{log}"
    );
}

#[test]
fn hooks_run_only_for_the_matching_result() {
    let mut runner = ScriptedRunner::default();
    let (report, log) = run(
        "# Job\n## A\n- actions: bash ok\n- if_fail: die\n## B\n- actions: bash next\n",
        &mut runner,
    );

    assert!(report.succeeded());
    assert_eq!(runner.calls, ["ok", "next"]);
    assert!(!log.contains("hook"), "log was:\n{log}");
}

#[test]
fn die_as_success_hook_stops_the_job() {
    let mut runner = ScriptedRunner::default();
    let (report, log) = run(
        "# Job\n## A\n- actions: bash ok\n- if_succeed: die\n## B\n- actions: bash next\n",
        &mut runner,
    );

    assert_eq!(report.outcome, JobOutcome::Died);
    assert_eq!(
        statuses(&report),
        [TaskStatus::Succeeded, TaskStatus::NotRun]
    );
    assert!(!report.succeeded());
    assert!(
        log.contains("[A] hook: die, stopping job"),
        "log was:\n{log}"
    );
    assert!(log.ends_with("job failed\n"), "log was:\n{log}");
}

#[test]
fn retry_as_success_hook_does_nothing() {
    let mut runner = ScriptedRunner::default();
    let (report, _) = run(
        "# Job\n## A\n- actions: bash ok\n- if_succeed: retry=2\n",
        &mut runner,
    );

    assert_eq!(runner.calls, ["ok"]);
    assert_eq!(report.tasks[0].attempts, 1);
    assert!(report.succeeded());
}

#[test]
fn task_without_actions_succeeds_and_passes_input_on() {
    let mut runner = ScriptedRunner::default();
    let (report, _) = run(
        "# Job\n## A\n- actions: bash a1\n## Empty\n## C\n- actions: bash c1\n",
        &mut runner,
    );

    assert_eq!(report.tasks[1].status, TaskStatus::Succeeded);
    assert_eq!(runner.inputs, ["", "a1 "]);
}

/// Writer that always fails.
struct BrokenWriter;

impl std::io::Write for BrokenWriter {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("log is closed"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn fails_when_the_log_cannot_be_written() {
    let job = parse_job("# Job\n## A\n- actions: bash ok\n").unwrap();
    let err = Executor::new(&job, DryRunner, BrokenWriter)
        .run()
        .unwrap_err();

    assert_eq!(err.to_string(), "log is closed");
}

#[test]
fn colors_only_failure_lines_red_when_color_is_on() {
    let job =
        parse_job("# Job\n## A\n- actions: mock fail\n- if_fail: retry=1\n## B\n- actions: mock\n")
            .unwrap();
    let mut log = Vec::new();
    Executor::new(&job, ActorRunner::default(), &mut log)
        .color(true)
        .run()
        .unwrap();
    let log = String::from_utf8(log).unwrap();

    let failed = "\x1b[31m[A] failed: `mock fail`: mock failure\x1b[0m\n";
    assert_eq!(log.matches(failed).count(), 2, "log was:\n{log}");
    assert_eq!(
        log.matches("\x1b[").count(),
        4,
        "only the 2 failure lines are colored"
    );
    assert!(log.contains("\n[A] retry 1 of 1\n"), "log was:\n{log}");
}

#[test]
fn does_not_color_by_default() {
    let job = parse_job("# Job\n## A\n- actions: mock fail\n").unwrap();
    let mut log = Vec::new();
    Executor::new(&job, ActorRunner::default(), &mut log)
        .run()
        .unwrap();
    let log = String::from_utf8(log).unwrap();

    assert!(log.contains("[A] failed: `mock fail`"), "log was:\n{log}");
    assert!(!log.contains('\x1b'), "log was:\n{log}");
}

#[test]
fn writes_the_clock_before_each_task_line_when_timestamps_are_on() {
    let job = parse_job(
        "# Job\n## A\n- actions: mock fail\n- if_fail: retry=1\n\
         ## B\n- actions: mock\n## C\n- begin_if: task(A) fail\n- actions: bash echo hi\n\
         - if_succeed: die\n",
    )
    .unwrap();
    let mut tick = 0;
    let mut log = Vec::new();
    Executor::new(&job, ActorRunner::default(), &mut log)
        .color(true)
        .timestamps(|| {
            tick += 1;
            format!("T{tick}")
        })
        .run()
        .unwrap();
    let log = String::from_utf8(log).unwrap();

    assert_eq!(
        log,
        "T1 [A] start\n\
         T2 [A] $ mock fail\n\
         T3 \x1b[31m[A] failed: `mock fail`: mock failure\x1b[0m\n\
         T4 [A] retry 1 of 1\n\
         T5 [A] $ mock fail\n\
         T6 \x1b[31m[A] failed: `mock fail`: mock failure\x1b[0m\n\
         T7 [B] skipped (begin_if not met)\n\
         T8 [C] start\n\
         T9 [C] $ bash echo hi\n\
         T10 [C] | hi\n\
         T11 [C] succeeded\n\
         T12 [C] hook: die, stopping job\n\
         job failed\n"
    );
}

#[test]
fn has_no_timestamps_by_default() {
    let job = parse_job("# Job\n## A\n- actions: mock\n").unwrap();
    let mut log = Vec::new();
    Executor::new(&job, ActorRunner::default(), &mut log)
        .run()
        .unwrap();
    let log = String::from_utf8(log).unwrap();

    assert_eq!(log, "[A] start\n[A] $ mock\n[A] succeeded\njob succeeded\n");
}

#[test]
fn actor_runner_runs_actions_with_and_without_debug() {
    let job = parse_job("# Job\n## A\n- actions:\n  - bash echo hi\n  - mock\n").unwrap();
    for debug in [false, true] {
        let mut log = Vec::new();
        let report = Executor::new(&job, ActorRunner::default().debug(debug), &mut log)
            .run()
            .unwrap();
        let log = String::from_utf8(log).unwrap();
        assert!(report.succeeded(), "log was:\n{log}");
        assert!(log.contains("[A] | hi"), "log was:\n{log}");
    }
}

// actions-parallel

#[test]
fn parallel_actions_get_the_same_input_and_pass_no_output_on() {
    let mut runner = ScriptedRunner::default();
    let (report, log) = run(
        "# Job\n\
         ## A\n- actions: bash first\n\
         ## B\n- actions-parallel:\n  - bash left\n  - bash right\n\
         ## C\n- actions: bash last\n",
        &mut runner,
    );
    assert!(report.succeeded(), "log was:\n{log}");
    assert_eq!(runner.calls, ["first", "left", "right", "last"]);
    // Both parallel actions get the output of A, and C gets no input.
    assert_eq!(runner.inputs, ["", "first ", "first ", ""]);
    assert!(
        log.contains(
            "[B] running 2 actions in parallel\n\
             [B] $ bash left\n\
             [B] | first left \n\
             [B] $ bash right\n\
             [B] | first right \n\
             [B] succeeded\n"
        ),
        "log was:\n{log}"
    );
}

#[test]
fn a_parallel_task_fails_with_the_errors_of_all_failed_actions() {
    let mut runner = ScriptedRunner::default().fail("left", 2).fail("right", 1);
    let (report, log) = run(
        "# Job\n\
         ## A\n- actions: bash first\n\
         ## B\n- actions-parallel:\n  - bash left\n  - bash right\n  - bash ok\n- if_fail: retry=1\n\
         ## C\n- begin_if: prev task fail\n- actions: bash last\n",
        &mut runner,
    );
    assert_eq!(
        report
            .tasks
            .iter()
            .map(|task| task.status)
            .collect::<Vec<_>>(),
        [
            TaskStatus::Succeeded,
            TaskStatus::Failed,
            TaskStatus::Succeeded
        ]
    );
    // A retry runs every action again, also the ones that succeeded.
    assert_eq!(runner.calls_to("ok"), 2);
    assert!(
        log.contains("[B] failed: `bash left`: scripted failure; `bash right`: scripted failure\n"),
        "log was:\n{log}"
    );
    assert!(
        log.contains("[B] failed: `bash left`: scripted failure\n"),
        "log was:\n{log}"
    );
    // Each error is right under the command of its own action.
    assert!(
        log.contains(
            "[B] $ bash left\n[B] ! scripted failure\n\
             [B] $ bash right\n[B] ! scripted failure\n[B] $ bash ok\n"
        ),
        "log was:\n{log}"
    );
    // A failed parallel task also passes no output on.
    assert_eq!(runner.inputs.last().unwrap(), "");
}

#[test]
fn actor_runner_runs_parallel_actions_at_the_same_time() {
    let job = parse_job(
        "# Job\n## A\n- actions-parallel:\n  - bash sleep 1\n  - bash sleep 1\n  - bash sleep 1\n  - bash printf done\n",
    )
    .unwrap();
    let mut log = Vec::new();
    let start = std::time::Instant::now();
    let report = Executor::new(&job, ActorRunner::default(), &mut log)
        .run()
        .unwrap();
    let elapsed = start.elapsed();
    let log = String::from_utf8(log).unwrap();
    assert!(report.succeeded(), "log was:\n{log}");
    assert!(log.contains("[A] | done\n"), "log was:\n{log}");
    // One after the other, the three sleeps take 3 seconds.
    assert!(elapsed.as_secs_f64() < 2.5, "took {elapsed:?}");
}

#[test]
fn actor_runner_reports_a_failed_parallel_action() {
    let mut runner = ActorRunner::default();
    let results = runner.run_parallel(
        &[
            "bash printf ok".parse().unwrap(),
            "mock fail".parse().unwrap(),
        ],
        "",
        None,
    );
    assert_eq!(
        results,
        [Ok("ok".to_string()), Err("mock failure".to_string())]
    );
    let mut debug = ActorRunner::default().debug(true);
    assert_eq!(
        debug.run_parallel(&["mock".parse().unwrap()], "in", Some(5)),
        [Ok("in".to_string())]
    );
}

/// Runs `job` with the real runner, and returns the report, the log and how
/// long the job took.
fn run_for_real(job: &str) -> (JobReport, String, std::time::Duration) {
    let job = parse_job(job).unwrap();
    let mut log = Vec::new();
    let start = std::time::Instant::now();
    let report = Executor::new(&job, ActorRunner::default(), &mut log)
        .run()
        .unwrap();
    let elapsed = start.elapsed();
    (report, String::from_utf8(log).unwrap(), elapsed)
}

#[test]
fn max_limits_how_many_parallel_actions_run_at_the_same_time() {
    let sleeps = "  - bash sleep 0.5\n".repeat(4);
    // At most one at a time: the four sleeps take 2 seconds.
    let (report, log, elapsed) =
        run_for_real(&format!("# Job\n## A\n- actions-parallel:max=1\n{sleeps}"));
    assert!(report.succeeded(), "log was:\n{log}");
    assert!(
        log.contains("[A] running 4 actions in parallel, at most 1 at a time\n"),
        "log was:\n{log}"
    );
    assert!(elapsed.as_secs_f64() >= 2.0, "took {elapsed:?}");

    // At most two at a time: two rounds of two sleeps take 1 second.
    let (report, log, elapsed) =
        run_for_real(&format!("# Job\n## A\n- actions-parallel:max=2\n{sleeps}"));
    assert!(report.succeeded(), "log was:\n{log}");
    let seconds = elapsed.as_secs_f64();
    assert!((1.0..1.9).contains(&seconds), "took {elapsed:?}");
}

#[test]
fn max_keeps_the_order_and_the_input_of_each_action() {
    // A limit larger than the number of actions runs all of them at once.
    let (report, log, _) = run_for_real(
        "# Job\n## A\n- actions: bash printf in\n\
         ## B\n- actions-parallel:max=8\n  - bash sed s/in/one/\n  - bash sed s/in/two/\n  - mock fail\n",
    );
    assert!(!report.succeeded());
    assert!(
        log.contains(
            "[B] $ bash sed s/in/one/\n[B] | one\n[B] $ bash sed s/in/two/\n[B] | two\n[B] $ mock fail\n[B] ! mock failure\n"
        ),
        "log was:\n{log}"
    );
    assert!(
        log.contains("[B] failed: `mock fail`: mock failure\n"),
        "log was:\n{log}"
    );
}

/// Makes an empty directory for one test of `tests/fixtures/concurrency.sh`.
fn concurrency_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("atama-concurrency-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Makes a job with task A, which has `parallel_field` and one action of
/// `tests/fixtures/concurrency.sh` for each name in `names`, and task B,
/// which prints the order in which the actions of A ended.
fn concurrency_job(
    dir: &std::path::Path,
    parallel_field: &str,
    names: &[&str],
    wait_for: usize,
) -> String {
    let dir = dir.display();
    let actions: String = names
        .iter()
        .map(|name| {
            format!("  - bash bash tests/fixtures/concurrency.sh {dir} {name} {wait_for}\n")
        })
        .collect();
    format!("# Job\n## A\n- {parallel_field}\n{actions}## B\n- actions: bash cat {dir}/order\n")
}

/// The number of actions that ran at the same time, as each action of
/// `tests/fixtures/concurrency.sh` saw it when it started.
fn peaks(dir: &std::path::Path) -> Vec<usize> {
    std::fs::read_to_string(dir.join("peaks"))
        .unwrap()
        .lines()
        .map(|line| line.trim().parse().unwrap())
        .collect()
}

#[test]
fn without_max_all_parallel_actions_run_at_the_same_time() {
    let dir = concurrency_dir("all");
    let names = ["a", "b", "c", "d", "e"];
    // Each action waits until all five started. If one action had to wait
    // for another to end, the five would never run together, and the
    // actions would fail.
    let (report, log, _) = run_for_real(&concurrency_job(&dir, "actions-parallel:", &names, 5));
    assert!(report.succeeded(), "log was:\n{log}");
    assert!(
        log.contains("[A] running 5 actions in parallel\n"),
        "log was:\n{log}"
    );
    assert_eq!(report.tasks[1].status, TaskStatus::Succeeded);
    let mut ended: Vec<String> = std::fs::read_to_string(dir.join("order"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    ended.sort();
    assert_eq!(ended, names);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn max_1_runs_one_action_at_a_time_and_then_goes_to_the_next_task() {
    let dir = concurrency_dir("max1");
    let names = ["a", "b", "c", "d"];
    let (report, log, _) =
        run_for_real(&concurrency_job(&dir, "actions-parallel:max=1", &names, 0));
    assert!(report.succeeded(), "log was:\n{log}");
    assert!(
        log.contains("[A] running 4 actions in parallel, at most 1 at a time\n"),
        "log was:\n{log}"
    );
    // Each action saw only itself running.
    assert_eq!(peaks(&dir), [1, 1, 1, 1]);
    // Each action ran once. With one at a time, they ran in the order of
    // the task.
    assert!(
        log.contains("[A] | a\n[A] $ bash bash tests/fixtures/concurrency.sh"),
        "log was:\n{log}"
    );
    // B started after all actions of A ended, and saw all of them.
    assert_eq!(
        report
            .tasks
            .iter()
            .map(|task| task.status)
            .collect::<Vec<_>>(),
        [TaskStatus::Succeeded, TaskStatus::Succeeded]
    );
    assert!(
        log.contains("[B] | a\n[B] | b\n[B] | c\n[B] | d\n"),
        "log was:\n{log}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn max_1_never_lets_two_actions_run_together() {
    let dir = concurrency_dir("max1-wait");
    // Each action waits until two actions started. With one at a time, the
    // second action starts only after the first one ends, so the first one
    // fails.
    let (report, log, _) = run_for_real(&concurrency_job(
        &dir,
        "actions-parallel:max=1",
        &["a", "b"],
        2,
    ));
    assert!(!report.succeeded(), "log was:\n{log}");
    assert!(
        log.contains("[A] failed: `bash bash tests/fixtures/concurrency.sh"),
        "log was:\n{log}"
    );
    assert!(
        log.contains("only 1 of 2 actions started"),
        "log was:\n{log}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn max_2_runs_two_actions_at_a_time() {
    let dir = concurrency_dir("max2");
    // Each action waits until two actions started, so two of them must run
    // at the same time.
    let (report, log, _) = run_for_real(&concurrency_job(
        &dir,
        "actions-parallel:max=2",
        &["a", "b", "c", "d"],
        2,
    ));
    assert!(report.succeeded(), "log was:\n{log}");
    let peaks = peaks(&dir);
    assert_eq!(peaks.len(), 4);
    assert!(peaks.iter().all(|&peak| peak <= 2), "peaks were {peaks:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runner that records the `max` that the executor gives to
/// [`ActionRunner::run_parallel`].
#[derive(Default)]
struct MaxRecorder {
    maxes: Vec<Option<usize>>,
}

impl ActionRunner for MaxRecorder {
    fn run(&mut self, _action: &Actor, input: &str) -> Result<String, String> {
        Ok(input.to_string())
    }

    fn run_parallel(
        &mut self,
        actions: &[Actor],
        input: &str,
        max: Option<usize>,
    ) -> Vec<Result<String, String>> {
        self.maxes.push(max);
        actions.iter().map(|_| Ok(input.to_string())).collect()
    }
}

#[test]
fn the_executor_gives_the_max_of_each_parallel_task_to_the_runner() {
    let mut runner = MaxRecorder::default();
    let (report, log) = run(
        "# Job\n\
         ## A\n- actions-parallel:\n  - mock\n  - mock\n\
         ## B\n- actions-parallel:max=1\n  - mock\n  - mock\n\
         ## C\n- actions-parallel:max=3\n  - mock\n\
         ## D\n- actions: mock\n",
        &mut runner,
    );
    assert!(report.succeeded(), "log was:\n{log}");
    // D has `actions`, so the executor does not call `run_parallel` for it.
    assert_eq!(runner.maxes, [None, Some(1), Some(3)]);
}
