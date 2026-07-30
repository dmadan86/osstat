//! ROADMAP M1's termination gate, end to end.
//!
//! Spawns a real child, finds it through the same provider the app uses, ends
//! it, and asserts it is gone. A unit test with a hand-built key would prove
//! the signalling works while skipping the part most likely to be wrong: that
//! the key the tree produces is the key the terminator accepts.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use osstat_core::{ProcessController, ProcessKey, ProcessProvider, Termination, TerminationMode};
use osstat_platform::{SysinfoSource, Terminator};

/// A spawned child, killed and reaped on drop.
///
/// A failing assertion partway through a test unwinds past whatever cleanup
/// code came after it. Wrapping the child so reaping happens in `Drop` means
/// every test cleans up its process on every path, including a panic, rather
/// than only on the success path a plain `Child` would cover.
struct ReapOnDrop(Child);

impl std::ops::Deref for ReapOnDrop {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for ReapOnDrop {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

/// Spawns a child that sleeps long enough to be found and ended.
///
/// Spawned directly rather than through `cmd /C`: `sysinfo::Process::kill`
/// shells out to `taskkill.exe /PID <pid> /F` on Windows, without `/T`, so a
/// `cmd /C ping ...` child would leave the `ping` grandchild it started
/// orphaned. A bare `ping` has no grandchildren to leak.
fn spawn_sleeper() -> ReapOnDrop {
    let child = if cfg!(target_os = "windows") {
        Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(Stdio::null())
            .spawn()
            .expect("ping should be on PATH on Windows")
    } else {
        Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep should be on PATH")
    };

    ReapOnDrop(child)
}

/// Finds a PID through the provider the app uses, with its start time.
///
/// Returns `None` on failure rather than panicking directly: `clippy::panic`
/// is denied workspace-wide (`-D warnings`), and the file-level `#![allow]`
/// above only covers `unwrap_used`/`expect_used`. Each call site turns a
/// `None` into an `.expect()`, which is the same failure through a lint-clean
/// shape.
fn key_for(pid: u32) -> Option<ProcessKey> {
    let mut source = SysinfoSource::new();

    for _ in 0..20 {
        let processes = source
            .processes()
            .expect("the process table should be readable");
        if let Some(found) = processes.iter().find(|process| process.pid == pid) {
            return Some(found.key());
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    None
}

/// Whether `child` has exited, waiting up to `timeout`.
fn exited_within(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    false
}

#[test]
fn a_spawned_child_is_found_in_the_tree_and_ended() {
    let mut child = spawn_sleeper();
    let key = key_for(child.id()).expect("the spawned child should appear in the process table");

    let mut terminator = Terminator::new();
    let outcome = terminator
        .terminate(key, TerminationMode::Forceful)
        .unwrap();

    assert_eq!(outcome, Termination::Signalled);
    assert!(
        exited_within(&mut child, Duration::from_secs(5)),
        "the child was signalled but never exited"
    );
}

#[test]
fn a_graceful_request_ends_a_child_that_has_no_reason_to_refuse() {
    // On Unix this is SIGTERM, which `sleep` honours. On Windows the console
    // child has no window, so the honest answer is NoWindowToClose and the
    // forceful step is what actually ends it — which is the behaviour the UI
    // relies on to skip a pointless five-second wait.
    let mut child = spawn_sleeper();
    let key = key_for(child.id()).expect("the spawned child should appear in the process table");
    let mut terminator = Terminator::new();

    let outcome = terminator
        .terminate(key, TerminationMode::Graceful)
        .unwrap();

    if cfg!(target_os = "windows") {
        assert_eq!(outcome, Termination::NoWindowToClose);
        terminator
            .terminate(key, TerminationMode::Forceful)
            .unwrap();
    } else {
        assert_eq!(outcome, Termination::Signalled);
    }

    assert!(exited_within(&mut child, Duration::from_secs(5)));
}

#[test]
fn a_reused_pid_is_refused_and_the_process_survives() {
    // The safety property, proven rather than asserted. The key carries a start
    // time one second off, standing in for a PID that was freed and reused.
    let mut child = spawn_sleeper();
    let real = key_for(child.id()).expect("the spawned child should appear in the process table");
    let stale = ProcessKey {
        pid: real.pid,
        started_at: real.started_at.wrapping_add(1),
    };

    let mut terminator = Terminator::new();
    let error = terminator
        .terminate(stale, TerminationMode::Forceful)
        .unwrap_err();

    assert!(matches!(error, osstat_core::Error::IdentityMismatch { .. }));
    // Deliberately generous: this is the assertion that makes the identity
    // check falsifiable, so it must not pass by accident. If the check were
    // broken and actually signalled, the forceful path on Windows shells out
    // to `taskkill.exe /PID … /F` — a process spawn and round trip that can
    // plausibly exceed a shorter window under load. A tight bound could let a
    // real signal through unnoticed just because `try_wait` had not caught up
    // yet, which would make this test decoration rather than proof.
    assert!(
        !exited_within(&mut child, Duration::from_secs(2)),
        "a stale key ended a process it did not identify"
    );

    terminator
        .terminate(real, TerminationMode::Forceful)
        .unwrap();
    let _ = child.wait();
}

#[test]
fn ending_something_already_gone_is_a_success() {
    let mut child = spawn_sleeper();
    let key = key_for(child.id()).expect("the spawned child should appear in the process table");
    let mut terminator = Terminator::new();

    terminator
        .terminate(key, TerminationMode::Forceful)
        .unwrap();
    assert!(exited_within(&mut child, Duration::from_secs(5)));
    let _ = child.wait();

    // Second call: the caller wanted it gone, and it is gone.
    let outcome = terminator
        .terminate(key, TerminationMode::Forceful)
        .unwrap();
    assert_eq!(outcome, Termination::AlreadyGone);
}
