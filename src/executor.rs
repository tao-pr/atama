//! Sequential job executor.
//!
//! Tasks run one at a time in the order they appear in the job. Before each
//! task its `begin_if` condition is checked against the tasks that already ran;
//! if it is not met the task is skipped. After a task finishes, its
//! `if_succeed` or `if_fail` hook runs.

use std::io::{self, Write};

use crate::job::{Action, Event, Hook, Job, Task, Trigger};
use crate::printer::{format_action, format_hook};

/// How many times a `restart` hook may restart a job before it gives up.
pub const DEFAULT_MAX_RESTARTS: u32 = 3;

/// Runs a single action.
pub trait ActionRunner {
    /// Runs the action, returning an error message if it failed.
    fn run(&mut self, action: &Action) -> Result<(), String>;
}

impl<R: ActionRunner + ?Sized> ActionRunner for &mut R {
    fn run(&mut self, action: &Action) -> Result<(), String> {
        (**self).run(action)
    }
}

/// Placeholder runner until actions are implemented: every action succeeds
/// without doing anything. The executor still logs each action it runs.
#[derive(Debug, Default)]
pub struct DryRunner;

impl ActionRunner for DryRunner {
    fn run(&mut self, _action: &Action) -> Result<(), String> {
        Ok(())
    }
}

/// Final state of a task after the job ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    /// The job stopped before reaching the task.
    NotRun,
    /// The task's `begin_if` condition was not met.
    Skipped,
    Succeeded,
    /// The task failed, after any retries.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskReport {
    pub id: String,
    pub status: TaskStatus,
    /// Number of times the task ran, including retries.
    pub attempts: u32,
}

/// How the job ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOutcome {
    /// Every task either ran or was skipped.
    Completed,
    /// A `die` hook stopped the job.
    Died,
    /// A `restart` hook fired after the restart limit was used up.
    RestartLimitReached,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobReport {
    pub outcome: JobOutcome,
    /// Number of times the job was restarted.
    pub restarts: u32,
    /// Task results from the last run through the job, in job order.
    pub tasks: Vec<TaskReport>,
}

impl JobReport {
    /// True if the job completed and no task ended in failure.
    pub fn succeeded(&self) -> bool {
        self.outcome == JobOutcome::Completed
            && self
                .tasks
                .iter()
                .all(|task| task.status != TaskStatus::Failed)
    }
}

/// What the job should do after a pass over its tasks.
enum Pass {
    Completed,
    Died,
    Restart,
}

/// Runs a job's tasks in order, writing a progress log to `out`.
pub struct Executor<'a, R, W> {
    job: &'a Job,
    runner: R,
    out: W,
    max_restarts: u32,
}

impl<'a, R: ActionRunner, W: Write> Executor<'a, R, W> {
    pub fn new(job: &'a Job, runner: R, out: W) -> Self {
        Executor {
            job,
            runner,
            out,
            max_restarts: DEFAULT_MAX_RESTARTS,
        }
    }

    /// Sets how many times a `restart` hook may restart the job.
    pub fn max_restarts(mut self, max_restarts: u32) -> Self {
        self.max_restarts = max_restarts;
        self
    }

    /// Runs the job. Only fails if writing the log fails.
    pub fn run(&mut self) -> io::Result<JobReport> {
        let mut restarts = 0;
        loop {
            let mut tasks: Vec<TaskReport> = self
                .job
                .tasks
                .iter()
                .map(|task| TaskReport {
                    id: task.id.clone(),
                    status: TaskStatus::NotRun,
                    attempts: 0,
                })
                .collect();

            let outcome = match self.run_pass(&mut tasks)? {
                Pass::Completed => JobOutcome::Completed,
                Pass::Died => JobOutcome::Died,
                Pass::Restart if restarts < self.max_restarts => {
                    restarts += 1;
                    writeln!(
                        self.out,
                        "restarting job ({restarts} of {})",
                        self.max_restarts
                    )?;
                    continue;
                }
                Pass::Restart => {
                    writeln!(
                        self.out,
                        "restart limit of {} reached, stopping job",
                        self.max_restarts
                    )?;
                    JobOutcome::RestartLimitReached
                }
            };

            let report = JobReport {
                outcome,
                restarts,
                tasks,
            };
            let result = if report.succeeded() {
                "succeeded"
            } else {
                "failed"
            };
            writeln!(self.out, "job {result}")?;
            return Ok(report);
        }
    }

    /// Runs every task once, in order, and reports what the job should do next.
    fn run_pass(&mut self, reports: &mut [TaskReport]) -> io::Result<Pass> {
        let job = self.job;
        for (index, task) in job.tasks.iter().enumerate() {
            if !should_begin(&task.begin_if, index, reports) {
                writeln!(self.out, "[{}] skipped (begin_if not met)", task.id)?;
                reports[index].status = TaskStatus::Skipped;
                continue;
            }

            let (status, attempts) = self.run_task(task)?;
            reports[index].status = status;
            reports[index].attempts = attempts;

            let hook = if status == TaskStatus::Succeeded {
                &task.if_succeed
            } else {
                &task.if_fail
            };
            match hook {
                None => {}
                Some(Hook::Die) => {
                    writeln!(self.out, "[{}] hook: die, stopping job", task.id)?;
                    return Ok(Pass::Died);
                }
                Some(Hook::Restart) => {
                    writeln!(self.out, "[{}] hook: restart", task.id)?;
                    return Ok(Pass::Restart);
                }
                // Retries already happened in `run_task`.
                Some(Hook::Retry(_)) => {}
                Some(hook) => {
                    writeln!(
                        self.out,
                        "[{}] hook: {} (not implemented yet)",
                        task.id,
                        format_hook(hook)
                    )?;
                }
            }
        }
        Ok(Pass::Completed)
    }

    /// Runs a task, retrying up to N times if its `if_fail` hook is `retry=N`.
    /// Returns the final status and the number of attempts.
    fn run_task(&mut self, task: &Task) -> io::Result<(TaskStatus, u32)> {
        let retries = match task.if_fail {
            Some(Hook::Retry(retries)) => retries,
            _ => 0,
        };

        for attempt in 0..=retries {
            if attempt == 0 {
                writeln!(self.out, "[{}] start", task.id)?;
            } else {
                writeln!(self.out, "[{}] retry {attempt} of {retries}", task.id)?;
            }

            match self.run_actions(task)? {
                Ok(()) => {
                    writeln!(self.out, "[{}] succeeded", task.id)?;
                    return Ok((TaskStatus::Succeeded, attempt + 1));
                }
                Err(message) => writeln!(self.out, "[{}] failed: {message}", task.id)?,
            }
        }
        Ok((TaskStatus::Failed, retries + 1))
    }

    /// Runs the task's actions in order, stopping at the first failure.
    fn run_actions(&mut self, task: &Task) -> io::Result<Result<(), String>> {
        for action in &task.actions {
            let command = format_action(action);
            writeln!(self.out, "[{}] $ {command}", task.id)?;
            if let Err(message) = self.runner.run(action) {
                return Ok(Err(format!("`{command}`: {message}")));
            }
        }
        Ok(Ok(()))
    }
}

/// Checks a task's `begin_if` condition against the tasks that already ran.
///
/// Tasks run in order, so a `task(id)` condition that points at a later task
/// is never met.
fn should_begin(trigger: &Trigger, index: usize, reports: &[TaskReport]) -> bool {
    match trigger {
        Trigger::PrevTask(event) => index == 0 || event_happened(reports[index - 1].status, *event),
        Trigger::Task { id, on } => reports
            .iter()
            .find(|report| report.id == *id)
            .is_some_and(|report| event_happened(report.status, *on)),
    }
}

fn event_happened(status: TaskStatus, event: Event) -> bool {
    match event {
        Event::Succeed => status == TaskStatus::Succeeded,
        Event::Fail => status == TaskStatus::Failed,
        Event::SucceedOrFail | Event::Begin => {
            matches!(status, TaskStatus::Succeeded | TaskStatus::Failed)
        }
    }
}
