//! Atama: automation pipeline as code, defined in markdown.

pub mod actor;
pub mod executor;
pub mod job;
pub mod parser;
pub mod passer;
pub mod printer;

pub use actor::Actor;
pub use executor::{
    ActionRunner, DryRunner, Executor, JobOutcome, JobReport, TaskReport, TaskStatus,
};
pub use job::{Action, Event, Hook, Job, Task, Trigger};
pub use parser::{ParseError, parse_job};
pub use printer::{format_job, print_error, print_job};
