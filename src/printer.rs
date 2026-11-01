//! Human-readable summary of a job for the terminal.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::IsTerminal;

use crate::actor::Actor;
use crate::job::{Hook, Job};

const BOLD: &str = "1";
const TASK_TITLE: &str = "1;36";
pub(crate) const FAIL: &str = "31";
const SUCCEED: &str = "32";
/// Black text on a light gray background.
const AGENT: &str = "30;47";

/// Format of [`local_timestamp`], for example `2026-10-24 09:15:02`.
pub const TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// The current local date and time, in [`TIMESTAMP_FORMAT`].
pub fn local_timestamp() -> String {
    chrono::Local::now().format(TIMESTAMP_FORMAT).to_string()
}

/// Whether to color output on `stream`: only when it is a terminal and
/// `NO_COLOR` is not set.
pub fn use_color(stream: &impl IsTerminal) -> bool {
    color_allowed(
        stream.is_terminal(),
        std::env::var_os("NO_COLOR").as_deref(),
    )
}

/// Whether to color output, given whether it goes to a terminal and the
/// value of `NO_COLOR`. An empty `NO_COLOR` counts as not set.
pub fn color_allowed(is_terminal: bool, no_color: Option<&OsStr>) -> bool {
    is_terminal && no_color.is_none_or(OsStr::is_empty)
}

/// Prints a summary of the job to stdout, in color when supported.
pub fn print_job(job: &Job) {
    print!("{}", format_job(job, use_color(&std::io::stdout())));
}

/// Prints `error: <message>` to stderr, in red when supported.
pub fn print_error(message: &str) {
    eprintln!("{}", format_error(message, use_color(&std::io::stderr())));
}

/// Formats `error: <message>`. With `color`, the text is red, using ANSI
/// escape codes.
pub fn format_error(message: &str, color: bool) -> String {
    paint(color, FAIL, &format!("error: {message}"))
}

/// Wraps `text` in the ANSI escape code `code` if `color` is set.
pub(crate) fn paint(color: bool, code: &str, text: &str) -> String {
    if color {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Formats the job name and one bullet per task, for example:
///
/// ```text
/// Job: Build and release
///   - Build: bash cargo build --release (retry 2x if fail) (write output to build.log if succeed)
///   - Cleanup:
/// ```
///
/// With `color`, task titles are cyan, `if fail` hooks red and
/// `if succeed` hooks green, and the `agent:<type>` word of an agent action
/// has a light background, using ANSI escape codes.
pub fn format_job(job: &Job, color: bool) -> String {
    let paint = |code: &str, text: &str| paint(color, code, text);

    let mut out = format!("{}\n", paint(BOLD, &format!("Job: {}", job.name)));
    for task in &job.tasks {
        let _ = write!(out, "  - {}", paint(TASK_TITLE, &format!("{}:", task.id)));

        let actions: Vec<String> = task
            .actions
            .iter()
            .map(|action| format_action(action, color))
            .collect();
        match (task.parallel, task.max_parallel) {
            (true, Some(max)) => {
                let _ = write!(out, " (parallel, max {max})");
            }
            (true, None) => {
                let _ = write!(out, " (parallel)");
            }
            (false, _) => {}
        }
        if !actions.is_empty() {
            let _ = write!(out, " {}", actions.join("; "));
        }
        if let Some(hook) = &task.if_fail {
            let _ = write!(
                out,
                " {}",
                paint(FAIL, &format!("({} if fail)", format_hook(hook)))
            );
        }
        if let Some(hook) = &task.if_succeed {
            let _ = write!(
                out,
                " {}",
                paint(SUCCEED, &format!("({} if succeed)", format_hook(hook)))
            );
        }
        out.push('\n');
    }
    out
}

/// Writes the action as its statement. With `color`, the `agent:<type>` word
/// of an agent action has a light background, so the reader sees which
/// steps run an agent and with which model. The agent of a `foreach` action
/// has the same background.
fn format_action(action: &Actor, color: bool) -> String {
    if let Actor::ForEach(inner) = action {
        return format!("foreach {}", format_action(inner, color));
    }
    let statement = action.to_string();
    let Actor::AgentTool { agent, .. } = action else {
        return statement;
    };
    let name = format!("agent:{agent}");
    let params = statement.strip_prefix(&name).unwrap_or_default();
    format!("{}{params}", paint(color, AGENT, &name))
}

pub(crate) fn format_hook(hook: &Hook) -> String {
    match hook {
        Hook::Retry(count) => format!("retry {count}x"),
        Hook::Die => "die".to_string(),
        Hook::Restart => "restart".to_string(),
        Hook::CallApi(arg) => format!("call API {arg}"),
        Hook::WriteFile(path) => format!("write output to {path}"),
        Hook::WriteErrorFile(path) => format!("write stderr to {path}"),
    }
}
