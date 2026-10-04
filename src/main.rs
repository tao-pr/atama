use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use atama::parse_job;
use clap::Parser;

/// Atama: automation pipeline as code, defined in markdown.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Path to the job configuration markdown file (e.g. job_config_file.md)
    job_config_file: PathBuf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let path = &cli.job_config_file;

    let input = match fs::read_to_string(path) {
        Ok(input) => input,
        Err(err) => {
            eprintln!("error: cannot read {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };

    match parse_job(&input) {
        Ok(job) => {
            println!("{job:#?}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {}: {err}", path.display());
            ExitCode::FAILURE
        }
    }
}
