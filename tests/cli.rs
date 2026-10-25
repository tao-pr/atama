//! Integration tests for the `atama` executable.
//!
//! Cargo builds the binary before running these tests and exposes its path
//! through the `CARGO_BIN_EXE_atama` environment variable.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use atama::TIMESTAMP_FORMAT;
use chrono::NaiveDateTime;

/// Runs the `atama` binary with the given arguments and returns its output.
/// It removes `TAVILY_API_KEY`, so that no test can use the real Tavily API
/// key of the shell that runs the tests.
fn run_atama(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_atama"))
        .args(args)
        .env_remove("TAVILY_API_KEY")
        .output()
        .expect("failed to run atama binary")
}

/// The line without its leading `YYYY-MM-DD HH:MM:SS ` timestamp, or `None`
/// if it does not start with one.
fn strip_timestamp(line: &str) -> Option<&str> {
    let (timestamp, rest) = line.split_at_checked(19)?;
    NaiveDateTime::parse_from_str(timestamp, TIMESTAMP_FORMAT).ok()?;
    rest.strip_prefix(' ')
}

/// The output with the timestamp removed from each line that has one.
fn strip_timestamps(output: &str) -> String {
    output
        .lines()
        .map(|line| strip_timestamp(line).unwrap_or(line))
        .flat_map(|line| [line, "\n"])
        .collect()
}

#[test]
fn fails_without_job_config_file() {
    let output = run_atama(&[]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("<JOB_CONFIG_FILE>"), "stderr was: {stderr}");
}

#[test]
fn parses_job_config_file() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/example_job.md");
    // The fixture runs cargo commands, so only do a dry run.
    let output = run_atama(&["--dry-run", fixture]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Build and release"), "stdout was: {stdout}");
}

#[test]
fn fails_on_missing_job_config_file() {
    let output = run_atama(&["does_not_exist.md"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot read"), "stderr was: {stderr}");
}

#[test]
fn prints_help() {
    let output = run_atama(&["--help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: atama"), "stdout was: {stdout}");
}

/// Creates an empty directory for one test and writes `job` to `job.md` in
/// it. Returns the directory and the path of the job file.
fn job_in_temp_dir(name: &str, job: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("atama-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let job = job.replace("{dir}", &dir.display().to_string());
    let path = dir.join("job.md");
    fs::write(&path, job).unwrap();
    (dir, path.display().to_string())
}

#[test]
fn runs_every_action_of_every_task() {
    let (dir, job) = job_in_temp_dir(
        "run",
        "# Job\n## Make\n- actions:\n  - bash mkdir -p {dir}/out\n  - bash touch {dir}/out/a.txt\n\
         ## Greet\n- actions: bash printf hello\n\
         ## List\n- actions: bash ls {dir}/out\n",
    );
    let output = run_atama(&[&job]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "stdout was: {stdout}");
    assert!(dir.join("out/a.txt").is_file());
    assert!(stdout.contains("[Greet] | hello"), "stdout was: {stdout}");
    assert!(stdout.contains("[List] | a.txt"), "stdout was: {stdout}");
    assert!(stdout.ends_with("job succeeded\n"), "stdout was: {stdout}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dry_run_runs_no_actions() {
    let (dir, job) = job_in_temp_dir("dry", "# Job\n## Make\n- actions: bash touch {dir}/a.txt\n");
    let output = run_atama(&["--dry-run", &job]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "stdout was: {stdout}");
    assert!(
        stdout.contains("[Make] $ bash touch"),
        "stdout was: {stdout}"
    );
    assert!(!dir.join("a.txt").exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fails_when_an_action_fails() {
    let (dir, job) = job_in_temp_dir("fail", "# Job\n## Broken\n- actions: mock fail\n");
    let output = run_atama(&[&job]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(!output.status.success(), "stdout was: {stdout}");
    assert!(
        stdout.contains("[Broken] failed: `mock fail`: mock failure"),
        "stdout was: {stdout}"
    );
    // stdout is a pipe here, not a terminal, so it has no color codes.
    assert!(!stdout.contains('\x1b'), "stdout was: {stdout}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn writes_a_timestamp_before_each_task_line() {
    let (dir, job) = job_in_temp_dir("timestamps", "# Job\n## A\n- actions: bash echo hi\n");
    let output = run_atama(&[&job]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    let task_lines: Vec<&str> = stdout.lines().filter_map(strip_timestamp).collect();
    assert_eq!(
        task_lines,
        [
            "[A] start",
            "[A] $ bash echo hi",
            "[A] | hi",
            "[A] succeeded"
        ],
        "stdout was: {stdout}"
    );
    // The job summary and the result line have no timestamp.
    assert!(stdout.starts_with("Job: Job\n"), "stdout was: {stdout}");
    assert!(
        stdout.ends_with("\njob succeeded\n"),
        "stdout was: {stdout}"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn debug_writes_agent_requests_to_stderr() {
    // Nothing listens on the port, so the agent fails after its first request.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let (dir, job) = job_in_temp_dir(
        "debug",
        &format!("# Job\n## Ask\n- actions: agent:coder model=m base_url={url} task=\"find it\"\n"),
    );

    let output = run_atama(&["--debug", &job]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains(&format!(
            "=== agent:coder configuration ===\nmodel: m\nbase_url: {url}\n"
        )),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("=== agent:coder step 1: request (2 messages, 2 new, tools: bash) ==="),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("\"content\": \"find it\""),
        "stderr was: {stderr}"
    );

    let output = run_atama(&[&job]);
    assert!(output.stderr.is_empty(), "stderr was: {:?}", output.stderr);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn tavily_search_sends_the_key_from_the_environment() {
    // A server that answers one request and sends back the raw request.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Read, Write};
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        let mut content_length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                content_length = value.trim().parse().unwrap();
            }
            request.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).unwrap();
        request.push_str(&String::from_utf8(body).unwrap());
        let reply = r#"{"results":[]}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
            reply.len()
        )
        .unwrap();
        request
    });
    let (dir, job) = job_in_temp_dir(
        "tavily",
        &format!("# Job\n## Search\n- actions: tavily_search base_url={url} max_results=2 rust\n"),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_atama"))
        .arg(&job)
        .env("TAVILY_API_KEY", "key-1")
        .output()
        .expect("failed to run atama binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "stdout was: {stdout}");
    assert!(
        stdout.contains(r#"| {"results":[]}"#),
        "stdout was: {stdout}"
    );
    assert!(!stdout.contains("key-1"), "stdout was: {stdout}");

    let request = server.join().unwrap();
    assert!(request.starts_with("POST /search "), "{request}");
    assert!(request.contains("authorization: Bearer key-1"), "{request}");
    assert!(
        request.ends_with(r#"{"query":"rust","max_results":2}"#),
        "{request}"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn strips_only_valid_timestamps() {
    assert_eq!(
        strip_timestamp("2026-10-24 09:15:02 [A] start"),
        Some("[A] start")
    );
    assert_eq!(strip_timestamp("2026-13-24 09:15:02 [A] start"), None);
    assert_eq!(strip_timestamp("2026-10-24 09:15:02[A] start"), None);
    assert_eq!(strip_timestamp("[A] start"), None);
    assert_eq!(strip_timestamps("2026-10-24 09:15:02 a\nb\n"), "a\nb\n");
}

#[test]
fn fails_on_invalid_job_config_file() {
    let (dir, job) = job_in_temp_dir("invalid", "# Job\n## A\n- actions: npm ci\n");
    let output = run_atama(&[&job]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(
        stderr.contains("job.md: line 3: unknown action `npm`"),
        "stderr was: {stderr}"
    );
    assert!(output.stdout.is_empty(), "nothing runs");
    fs::remove_dir_all(dir).unwrap();
}

/// Runs the `atama` binary with its stdout and stderr on a pseudo-terminal,
/// so it behaves as in a real terminal. Sets the environment variables in
/// `envs` and removes `NO_COLOR` unless `envs` sets it. Returns everything
/// that the binary printed, with `\r\n` changed to `\n`.
#[cfg(unix)]
fn run_atama_in_terminal(args: &[&str], envs: &[(&str, &str)]) -> String {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let (mut master, mut slave) = (0, 0);
    // SAFETY: openpty writes two new file descriptors into `master` and
    // `slave`. The null pointers mean: no name, default settings and size.
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(result, 0, "cannot open a pseudo-terminal");
    // SAFETY: openpty succeeded, so both are open descriptors that we own.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };

    let mut command = Command::new(env!("CARGO_BIN_EXE_atama"));
    command
        .args(args)
        .env_remove("NO_COLOR")
        .envs(envs.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave.try_clone().unwrap()));
    let mut child = command.spawn().expect("failed to run atama binary");
    drop(command);

    // Keep `slave` open until all output is read. On macOS, output that is
    // not read yet is lost when the last descriptor of the terminal closes.
    let mut master = std::fs::File::from(master);
    let mut poll = libc::pollfd {
        fd: master.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut output = Vec::new();
    let mut buffer = [0; 4096];
    let error = loop {
        if Instant::now() > deadline {
            break Some("atama did not finish in time".to_string());
        }
        // SAFETY: `poll` points to one valid pollfd for this call.
        let ready = unsafe { libc::poll(&mut poll, 1, 100) };
        if ready > 0 {
            match master.read(&mut buffer) {
                Ok(count) => output.extend_from_slice(&buffer[..count]),
                Err(err) => break Some(format!("cannot read the terminal: {err}")),
            }
        } else if matches!(child.try_wait(), Ok(Some(_))) {
            // The binary exited and printed nothing for 100 ms.
            break None;
        }
    };
    // Always collect the process, so it does not stay as a zombie.
    let _ = child.kill();
    child.wait().expect("cannot wait for atama");
    if let Some(error) = error {
        panic!("{error}");
    }
    drop(slave);
    String::from_utf8(output).unwrap().replace("\r\n", "\n")
}

#[cfg(unix)]
#[test]
fn colors_failure_lines_red_in_a_terminal() {
    let (dir, job) = job_in_temp_dir(
        "terminal",
        "# Job\n## Broken\n- actions: mock fail\n## Next\n- actions: mock\n",
    );
    let output = strip_timestamps(&run_atama_in_terminal(&[&job], &[]));

    assert!(
        output.contains("\n\x1b[31m[Broken] failed: `mock fail`: mock failure\x1b[0m\n"),
        "output was: {output:?}"
    );
    assert!(
        output.contains("\n[Broken] start\n"),
        "output was: {output:?}"
    );
    assert!(
        output.contains("\n[Next] skipped (begin_if not met)\n"),
        "output was: {output:?}"
    );
    assert!(output.ends_with("\njob failed\n"), "output was: {output:?}");
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn no_color_turns_off_colors_in_a_terminal() {
    let (dir, job) = job_in_temp_dir("no_color", "# Job\n## Broken\n- actions: mock fail\n");
    let output = strip_timestamps(&run_atama_in_terminal(&[&job], &[("NO_COLOR", "1")]));

    assert!(
        output.contains("\n[Broken] failed: `mock fail`: mock failure\n"),
        "output was: {output:?}"
    );
    assert!(!output.contains('\x1b'), "output was: {output:?}");
    fs::remove_dir_all(dir).unwrap();
}
