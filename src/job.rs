//! Data model for a job (pipeline) definition.

/// A pipeline parsed from a job config markdown file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// Text of the `# <name>` heading.
    pub name: String,
    /// Text between the job heading and the first task.
    pub description: String,
    /// Tasks in the order they appear in the file.
    pub tasks: Vec<Task>,
}

/// A single `## <id>` section of a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// Text of the `## <id>` heading, used to refer to it in `task(id)` triggers.
    pub id: String,
    /// Free text in the task section that is not a field.
    pub description: String,
    /// When the task starts. Defaults to `prev task succeed`.
    pub begin_if: Trigger,
    /// Actions to run, in order.
    pub actions: Vec<String>,
    /// Hook to run when the task fails.
    pub if_fail: Option<Hook>,
    /// Hook to run when the task succeeds.
    pub if_succeed: Option<Hook>,
}

/// A `begin_if` trigger condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// `prev task <event>`. Never holds [`Event::Begin`].
    PrevTask(Event),
    /// `task(<id>) <event>`.
    Task { id: String, on: Event },
}

impl Default for Trigger {
    fn default() -> Self {
        Trigger::PrevTask(Event::Succeed)
    }
}

/// The task event a trigger waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Succeed,
    Fail,
    SucceedOrFail,
    Begin,
}

/// An `if_fail` / `if_succeed` hook action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hook {
    /// `retry=N`: retry the current task at most N times.
    Retry(u32),
    /// `die`: skip all tasks that have not run yet.
    Die,
    /// `restart`: restart the whole job from the first task.
    Restart,
    /// `call_api(...)`: the raw argument text, not interpreted yet.
    CallApi(String),
    /// `write_file(...)`: write the output to a file.
    WriteFile(String),
    /// `write_error_file(...)`: write stderr to a file.
    WriteErrorFile(String),
}
