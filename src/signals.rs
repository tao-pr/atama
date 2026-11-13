//! Stops the commands of Atama when Atama gets a termination signal.
//!
//! Each command that [`crate::actor::run_process`] starts runs in its own
//! process group, so a signal to the group reaches the command and all its
//! child processes. [`register`] adds a group to a list of running groups,
//! and the [`Registered`] guard removes it when the command ends.
//!
//! [`install`] handles `SIGINT`, `SIGTERM` and `SIGHUP`. The signal handler
//! only writes the signal number to a pipe. A watcher thread reads it, and
//! calls [`terminate_groups`]: it sends the same signal to each running
//! group, gives the groups [`GRACE`] to stop, and kills the groups that
//! still run. Then Atama stops with the same signal.
//!
//! After the signal, Atama is stopping (see [`stopping`]). A thread that is
//! about to start a command, and `main` when the job ends, wait for the
//! watcher thread to stop Atama (see [`wait_if_stopping`]). So no new
//! command or task starts, and the exit status names the signal.

use std::time::Duration;

/// The time that the commands get to stop after Atama passes a signal on,
/// before Atama kills them.
pub const GRACE: Duration = Duration::from_secs(2);

/// The most process groups that the list holds. A command that starts when
/// the list is full is not on the list, and a signal does not reach it.
pub const SLOTS: usize = 256;

#[cfg(unix)]
mod unix {
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::SLOTS;

    /// The ids of the running process groups. 0 is a free slot.
    static GROUPS: [AtomicI32; SLOTS] = [const { AtomicI32::new(0) }; SLOTS];

    /// The write end of the pipe of the signal handler, or -1.
    static PIPE: AtomicI32 = AtomicI32::new(-1);

    /// Whether Atama got a termination signal.
    static STOPPING: AtomicBool = AtomicBool::new(false);

    /// Whether Atama got a termination signal and is stopping.
    pub fn stopping() -> bool {
        STOPPING.load(Ordering::SeqCst)
    }

    /// If Atama is stopping, waits until the watcher thread stops Atama,
    /// and never returns. Else returns at once.
    pub fn wait_if_stopping() {
        if stopping() {
            wait_forever();
        }
    }

    /// Waits until the watcher thread stops Atama.
    fn wait_forever() -> ! {
        loop {
            thread::park();
        }
    }

    /// Removes its process group from the list when it is dropped.
    #[derive(Debug)]
    pub struct Registered {
        slot: Option<usize>,
    }

    impl Drop for Registered {
        fn drop(&mut self) {
            if let Some(slot) = self.slot {
                GROUPS[slot].store(0, Ordering::SeqCst);
            }
        }
    }

    /// Adds the process group `group` to the list of running groups, until
    /// the guard is dropped. If the list is full, the group is not on it.
    pub fn register(group: i32) -> Registered {
        let slot = GROUPS.iter().position(|slot| {
            slot.compare_exchange(0, group, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        });
        Registered { slot }
    }

    /// The ids of the running process groups on the list.
    pub fn running_groups() -> Vec<i32> {
        GROUPS
            .iter()
            .map(|slot| slot.load(Ordering::SeqCst))
            .filter(|group| *group != 0)
            .collect()
    }

    /// Sends `signal` to each running process group, waits up to `grace`
    /// until all groups stopped, and kills the groups that still run with
    /// `SIGKILL`. Returns the number of groups that Atama killed.
    ///
    /// Each pass reads the list again, and sends `signal` to a group that
    /// is new on it. So a command that started just before the signal also
    /// stops.
    pub fn terminate_groups(signal: i32, grace: Duration) -> usize {
        let end = Instant::now() + grace;
        let mut seen = Vec::new();
        let mut alive = Vec::new();
        loop {
            for group in running_groups() {
                if !seen.contains(&group) {
                    // SAFETY: `kill` only sends a signal. A negative id sends
                    // it to the process group that Atama made for one command.
                    unsafe { libc::kill(-group, signal) };
                    seen.push(group);
                    alive.push(group);
                }
            }
            // SAFETY: signal 0 only asks whether the group still exists.
            alive.retain(|group| unsafe { libc::kill(-group, 0) } == 0);
            if alive.is_empty() || Instant::now() >= end {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        for group in &alive {
            // SAFETY: as above.
            unsafe { libc::kill(-group, libc::SIGKILL) };
        }
        alive.len()
    }

    /// Handles `SIGINT`, `SIGTERM` and `SIGHUP` (see the module docs). Call
    /// it once, at the start of `main`.
    ///
    /// If Atama cannot make the pipe, it does not handle the signals.
    pub fn install() {
        let mut fds = [0; 2];
        // SAFETY: `fds` has room for the two file descriptors.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == 0 {
            install_with(fds);
        }
    }

    /// Starts the watcher thread on the pipe `fds`, and sets the handler.
    fn install_with([read_end, write_end]: [i32; 2]) {
        PIPE.store(write_end, Ordering::SeqCst);
        thread::spawn(move || {
            let signal = read_signal(read_end);
            terminate_groups(signal, super::GRACE);
            #[cfg(coverage)]
            write_coverage();
            // SAFETY: the default action of the signal stops Atama with it,
            // so that the exit status names the signal.
            unsafe {
                libc::signal(signal, libc::SIG_DFL);
                libc::raise(signal);
            }
        });
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            // SAFETY: `handle` only calls `write`, which is safe in a signal
            // handler.
            unsafe { libc::signal(signal, handle as *const () as libc::sighandler_t) };
        }
    }

    /// Writes the coverage data of a coverage build (`cargo llvm-cov`). A
    /// process that a signal stops does not write it at exit, so the tests
    /// of the signals would not count.
    #[cfg(coverage)]
    fn write_coverage() {
        unsafe extern "C" {
            fn __llvm_profile_write_file() -> i32;
        }
        // SAFETY: the function of the coverage runtime takes no arguments.
        unsafe { __llvm_profile_write_file() };
    }

    /// Reads one signal number from the pipe. A read that a signal
    /// interrupts starts again.
    fn read_signal(read_end: i32) -> i32 {
        let mut byte = 0u8;
        // SAFETY: `byte` has room for the one byte that `read` writes.
        while unsafe { libc::read(read_end, (&raw mut byte).cast(), 1) } != 1 {}
        i32::from(byte)
    }

    /// The signal handler: marks Atama as stopping, and writes the signal
    /// number to the pipe.
    extern "C" fn handle(signal: libc::c_int) {
        STOPPING.store(true, Ordering::SeqCst);
        let byte = signal as u8;
        // SAFETY: `write` is safe in a signal handler, and `byte` lives
        // until it returns.
        unsafe { libc::write(PIPE.load(Ordering::SeqCst), (&raw const byte).cast(), 1) };
    }
}

#[cfg(unix)]
pub use unix::{
    Registered, install, register, running_groups, stopping, terminate_groups, wait_if_stopping,
};

/// On other systems, commands have no process groups, and Atama does not
/// handle signals.
#[cfg(not(unix))]
mod other {
    /// Does nothing on this system.
    #[derive(Debug)]
    pub struct Registered;

    pub fn register(_group: i32) -> Registered {
        Registered
    }

    pub fn install() {}

    pub fn stopping() -> bool {
        false
    }

    pub fn wait_if_stopping() {}
}

#[cfg(not(unix))]
pub use other::{Registered, install, register, stopping, wait_if_stopping};
