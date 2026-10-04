//! Atama: automation pipeline as code, defined in markdown.

pub mod job;
pub mod parser;

pub use job::{Event, Hook, Job, Task, Trigger};
pub use parser::{ParseError, parse_job};
