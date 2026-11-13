//! Tests for `src/signals.rs`: the list of running process groups, and how
//! Atama stops them.
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use atama::signals::{GRACE, SLOTS, register, running_groups, terminate_groups};

/// The list of groups is global, so the tests of this file run one at a
/// time.
static LOCK: Mutex<()> = Mutex::new(());

/// Whether the process `pid` still runs, after up to 3 seconds of waiting
/// for it to stop.
fn still_runs(pid: i32) -> bool {
    for _ in 0..300 {
        // SAFETY: signal 0 only asks whether the process exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    true
}

/// Starts `script` in bash in its own process group, on the list. The
/// script prints the id of a child process on its first line. Returns that
/// id. Another thread waits for bash, so that it does not stay a zombie.
fn start_group(script: &str) -> (i32, atama::signals::Registered) {
    let mut child = Command::new("bash")
        .args(["-c", script])
        .process_group(0)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let registered = register(child.id() as i32);
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    thread::spawn(move || child.wait());
    (line.trim().parse().unwrap(), registered)
}

#[test]
fn a_group_is_on_the_list_while_its_guard_lives() {
    let _lock = LOCK.lock().unwrap();
    let guard = register(4_000_001);
    assert!(running_groups().contains(&4_000_001));
    drop(guard);
    assert!(!running_groups().contains(&4_000_001));
}

#[test]
fn a_group_is_not_on_a_full_list() {
    let _lock = LOCK.lock().unwrap();
    let guards: Vec<_> = (0..SLOTS as i32).map(|i| register(4_100_000 + i)).collect();
    let extra = register(4_000_002);
    assert!(!running_groups().contains(&4_000_002));
    drop(extra);
    assert_eq!(running_groups().len(), SLOTS);
    drop(guards);
    assert!(running_groups().is_empty());
}

#[test]
fn passes_the_signal_on_to_the_child_processes() {
    let _lock = LOCK.lock().unwrap();
    let (child, _registered) = start_group("sleep 30 & echo $!; wait");
    let start = Instant::now();
    assert_eq!(terminate_groups(libc::SIGTERM, GRACE), 0);
    assert!(start.elapsed() < GRACE, "{:?}", start.elapsed());
    assert!(!still_runs(child));
}

#[test]
fn kills_a_group_that_ignores_the_signal() {
    let _lock = LOCK.lock().unwrap();
    let (child, _registered) = start_group("trap '' TERM; sleep 30 & echo $!; wait");
    assert_eq!(
        terminate_groups(libc::SIGTERM, Duration::from_millis(300)),
        1
    );
    assert!(!still_runs(child));
}
