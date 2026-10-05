//! Atama: automation pipeline as code, defined in markdown.

pub mod job;
pub mod parser;
pub mod printer;

pub use job::{Action, Event, Hook, Job, Task, Trigger};
pub use parser::{ParseError, parse_job};
pub use printer::{format_job, print_job};
