use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use atama::{
    ActionRunner, ActorRunner, DryRunner, Executor, Job, parse_job, print_error, print_job,
    use_color,
};
use clap::Parser;

/// Atama: automation pipeline as code, defined in markdown.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Path to the job configuration markdown file (e.g. job_config_file.md)
    job_config_file: PathBuf,

    /// Print the job and log each action without running it
    #[arg(long)]
    dry_run: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let path = &cli.job_config_file;

    let input = match fs::read_to_string(path) {
        Ok(input) => input,
        Err(err) => {
            print_error(&format!("cannot read {}: {err}", path.display()));
            return ExitCode::FAILURE;
        }
    };

    let job = match parse_job(&input) {
        Ok(job) => job,
        Err(err) => {
            print_error(&format!("{}: {err}", path.display()));
            return ExitCode::FAILURE;
        }
    };

    print_job(&job);
    println!();

    if cli.dry_run {
        execute(&job, DryRunner)
    } else {
        execute(&job, ActorRunner)
    }
}

/// Runs the job with `runner` and returns the exit code for its result.
fn execute(job: &Job, runner: impl ActionRunner) -> ExitCode {
    let stdout = io::stdout();
    let color = use_color(&stdout);
    match Executor::new(job, runner, stdout.lock()).color(color).run() {
        Ok(report) if report.succeeded() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(err) => {
            print_error(&format!("cannot write output: {err}"));
            ExitCode::FAILURE
        }
    }
}
