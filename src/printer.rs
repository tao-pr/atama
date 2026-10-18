//! Human-readable summary of a job for the terminal.

use std::fmt::Write as _;
use std::io::IsTerminal;

use crate::job::{Hook, Job};

const BOLD: &str = "1";
const TASK_TITLE: &str = "1;36";
const FAIL: &str = "31";
const SUCCEED: &str = "32";

/// Whether to color output on `stream`: only when it is a terminal and
/// `NO_COLOR` is not set.
fn use_color(stream: &impl IsTerminal) -> bool {
    stream.is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
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
    let line = format!("error: {message}");
    if color {
        format!("\x1b[{FAIL}m{line}\x1b[0m")
    } else {
        line
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
/// `if succeed` hooks green, using ANSI escape codes.
pub fn format_job(job: &Job, color: bool) -> String {
    let paint = |code: &str, text: &str| {
        if color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    };

    let mut out = format!("{}\n", paint(BOLD, &format!("Job: {}", job.name)));
    for task in &job.tasks {
        let _ = write!(out, "  - {}", paint(TASK_TITLE, &format!("{}:", task.id)));

        let actions: Vec<String> = task.actions.iter().map(ToString::to_string).collect();
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
