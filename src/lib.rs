//! Atama: automation pipeline as code, defined in markdown.

pub mod actor;
pub mod agent_loop_openai_compat;
pub mod executor;
pub mod job;
pub mod parser;
pub mod passer;
pub mod printer;

pub use actor::Actor;
pub use agent_loop_openai_compat::AgentToolLoop;
pub use executor::{
    ActionRunner, ActorRunner, DryRunner, Executor, JobOutcome, JobReport, TaskReport, TaskStatus,
};
pub use job::{Event, Hook, Job, Task, Trigger};
pub use parser::{ParseError, parse_job};
pub use passer::Passer;
pub use printer::{
    TIMESTAMP_FORMAT, color_allowed, format_error, format_job, local_timestamp, print_error,
    print_job, use_color,
};
