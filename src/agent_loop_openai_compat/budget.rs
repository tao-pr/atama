//! The limits of one run of the agent loop, and what the run used.
//!
//! A step is one request to the model and its one reply. An iteration is
//! one attempt at the whole task. It ends when the agent replies without
//! tool calls. `max_steps` counts the steps of the whole run, in all
//! iterations. `coder` and `summarizer` runs have one iteration.

use std::time::{Duration, Instant};

use super::agent_options::format_duration;

/// The limits of one run, and what the run used of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunBudget {
    max_iterations: u32,
    max_steps: u32,
    token_budget: Option<u64>,
    /// The start of the run and its most wall clock time, if set.
    time_limit: Option<(Instant, Duration)>,
    iterations: u32,
    steps: u32,
    tokens: u64,
}

/// Why a run must stop before it has an output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// The run used all its steps.
    Steps { max: u32 },
    /// The run used its token budget.
    Tokens { used: u64, limit: u64, steps: u32 },
    /// A token budget is set, but a reply has no token count.
    MissingTokens { limit: u64 },
    /// The run used all its iterations.
    Iterations { max: u32 },
    /// The run used its time limit.
    Time { max: Duration },
}

impl RunBudget {
    /// Makes the budget of a run that is in its first iteration and used
    /// nothing. `token_budget` `None` sets no token limit.
    pub fn new(max_iterations: u32, max_steps: u32, token_budget: Option<u64>) -> Self {
        RunBudget {
            max_iterations,
            max_steps,
            token_budget,
            time_limit: None,
            iterations: 1,
            steps: 0,
            tokens: 0,
        }
    }

    /// Sets a time limit: the run can work for `max` from `start`.
    pub fn with_time_limit(mut self, start: Instant, max: Duration) -> Self {
        self.time_limit = Some((start, max));
        self
    }

    /// Fails if the run used its time limit at `now`.
    pub fn check_time(&self, now: Instant) -> Result<(), StopReason> {
        match self.time_limit {
            Some((start, max)) if now.saturating_duration_since(start) >= max => {
                Err(StopReason::Time { max })
            }
            _ => Ok(()),
        }
    }

    /// The end of the time limit, if a limit is set.
    pub fn deadline(&self) -> Option<Instant> {
        self.time_limit.map(|(start, max)| start + max)
    }

    /// The time that the run used at `now`, and its time limit, if a limit
    /// is set.
    pub fn time_usage(&self, now: Instant) -> Option<(Duration, Duration)> {
        self.time_limit
            .map(|(start, max)| (now.saturating_duration_since(start), max))
    }

    /// Starts the next step, and returns its number, from 1.
    ///
    /// Fails if the run used all its steps, or used its token budget. The
    /// step limit comes first.
    pub fn start_step(&mut self) -> Result<u32, StopReason> {
        if self.steps >= self.max_steps {
            return Err(StopReason::Steps {
                max: self.max_steps,
            });
        }
        if let Some(limit) = self.token_budget
            && self.tokens >= limit
        {
            return Err(StopReason::Tokens {
                used: self.tokens,
                limit,
                steps: self.steps,
            });
        }
        self.steps += 1;
        Ok(self.steps)
    }

    /// Adds the tokens of a reply.
    ///
    /// Fails if a token budget is set and `tokens` is `None`, because then
    /// the run cannot count its tokens. Without a token budget, `None` adds
    /// nothing.
    pub fn add_tokens(&mut self, tokens: Option<u64>) -> Result<(), StopReason> {
        match (tokens, self.token_budget) {
            (Some(tokens), _) => {
                self.tokens += tokens;
                Ok(())
            }
            (None, Some(limit)) => Err(StopReason::MissingTokens { limit }),
            (None, None) => Ok(()),
        }
    }

    /// Starts the next iteration, and returns its number, from 1.
    ///
    /// Fails if the run used all its iterations.
    pub fn next_iteration(&mut self) -> Result<u32, StopReason> {
        if self.iterations >= self.max_iterations {
            return Err(StopReason::Iterations {
                max: self.max_iterations,
            });
        }
        self.iterations += 1;
        Ok(self.iterations)
    }

    /// The number of the current iteration, from 1.
    pub fn iteration(&self) -> u32 {
        self.iterations
    }

    /// The tokens that the run used, and the token budget, if a budget is
    /// set.
    pub fn token_usage(&self) -> Option<(u64, u64)> {
        self.token_budget.map(|limit| (self.tokens, limit))
    }
}

impl StopReason {
    /// The error of the run. `label` names the agent (see
    /// [`crate::AgentToolLoop::label`]).
    pub fn message(&self, label: &str) -> String {
        match self {
            StopReason::Steps { max } => format!("`{label}` did not finish in {max} steps"),
            StopReason::Tokens { used, limit, steps } => format!(
                "`{label}` used {used} tokens in {steps} steps, and its token budget is {limit}"
            ),
            StopReason::MissingTokens { limit } => format!(
                "`{label}` has a token budget of {limit}, but the server did not send \
                 `usage.total_tokens` in its response"
            ),
            StopReason::Iterations { max } => {
                format!("`{label}` did not finish in {max} iterations")
            }
            StopReason::Time { max } => format!(
                "`{label}` did not finish in its time limit of {}",
                format_duration(*max)
            ),
        }
    }
}
