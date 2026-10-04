//! Tests for the job config markdown parser.

use atama::{Event, Hook, Trigger, parse_job};

const EXAMPLE_JOB: &str = include_str!("fixtures/example_job.md");

#[test]
fn parses_example_job() {
    let job = parse_job(EXAMPLE_JOB).expect("example job should parse");

    assert_eq!(job.name, "Build and release");
    assert_eq!(
        job.description,
        "Builds the project, runs the tests and publishes a release."
    );
    let ids: Vec<&str> = job.tasks.iter().map(|task| task.id.as_str()).collect();
    assert_eq!(ids, ["Build", "Test", "Lint", "Release", "Cleanup"]);

    let build = &job.tasks[0];
    assert_eq!(build.description, "Compile the release binary.");
    assert_eq!(build.begin_if, Trigger::PrevTask(Event::Succeed));
    assert_eq!(build.actions, ["cargo build --release"]);
    assert_eq!(build.if_fail, Some(Hook::Retry(2)));
    assert_eq!(build.if_succeed, Some(Hook::WriteFile("build.log".into())));

    let lint = &job.tasks[2];
    assert_eq!(
        lint.begin_if,
        Trigger::Task {
            id: "Test".into(),
            on: Event::Begin
        }
    );
    assert_eq!(lint.if_fail, Some(Hook::Die));

    let cleanup = &job.tasks[4];
    assert_eq!(cleanup.begin_if, Trigger::PrevTask(Event::SucceedOrFail));
    assert!(cleanup.actions.is_empty());
}

#[test]
fn parses_trigger_conditions() {
    let cases = [
        ("prev task succeed", Trigger::PrevTask(Event::Succeed)),
        ("prev task fail", Trigger::PrevTask(Event::Fail)),
        ("prev task succeed or fail", Trigger::PrevTask(Event::SucceedOrFail)),
        (
            "task(Build) succeed or fail",
            Trigger::Task {
                id: "Build".into(),
                on: Event::SucceedOrFail,
            },
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(input.parse::<Trigger>(), Ok(expected), "input: {input}");
    }

    assert!("prev task begin".parse::<Trigger>().is_err());
    assert!("task() succeed".parse::<Trigger>().is_err());
    assert!("whenever".parse::<Trigger>().is_err());
}

#[test]
fn parses_hooks() {
    let cases = [
        ("retry=3", Hook::Retry(3)),
        ("die", Hook::Die),
        ("restart", Hook::Restart),
        ("call_api(https://x.test)", Hook::CallApi("https://x.test".into())),
        ("write_file(out.txt)", Hook::WriteFile("out.txt".into())),
        ("write_error_file(err.txt)", Hook::WriteErrorFile("err.txt".into())),
    ];
    for (input, expected) in cases {
        assert_eq!(input.parse::<Hook>(), Ok(expected), "input: {input}");
    }

    assert!("retry=many".parse::<Hook>().is_err());
    assert!("write_file(out.txt".parse::<Hook>().is_err());
    assert!("explode".parse::<Hook>().is_err());
}

#[test]
fn ignores_headings_inside_code_blocks() {
    let input = "# Job\n\n## Task\n\n```sh\n# not a heading\n## not a task\n```\n";
    let job = parse_job(input).expect("job should parse");

    assert_eq!(job.tasks.len(), 1);
    assert!(job.tasks[0].description.contains("## not a task"));
}

/// Asserts that parsing fails on `line` with a message containing `message`.
fn assert_error(input: &str, line: usize, message: &str) {
    let err = parse_job(input).expect_err("input should not parse");
    assert_eq!(err.line, line, "error was: {err}");
    assert!(err.message.contains(message), "error was: {err}");
}

#[test]
fn rejects_invalid_jobs() {
    assert_error("", 1, "missing `# <job name>`");
    assert_error("some text\n# Job\n", 1, "expected the `# <job name>`");
    assert_error("# Job\n# Other\n", 2, "more than one");
    assert_error("# Job\n", 1, "no `## <task>`");
    assert_error("# Job\n## A\n## A\n", 3, "duplicate task id `A`");
    assert_error("# Job\n## A\n- begin_if: prev task fail\n", 3, "first task");
    assert_error("# Job\n## A\n## B\n- begin_if: task(C) fail\n", 4, "unknown task id `C`");
    assert_error("# Job\n## A\n## B\n- begin_if: task(B) fail\n", 4, "depends on itself");
    assert_error("# Job\n## A\n- if_fail: retry=1\n- if_fail: die\n", 4, "duplicate `if_fail`");
    assert_error("# Job\n## A\n- if_succeed:\n", 3, "has no value");
}
