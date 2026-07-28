//! The M1 performance gate: a 500-process refresh in under 50 ms.
//!
//! The roadmap commits to that number, and it is committed here rather than
//! measured by hand because a regression shows up as a janky UI — easy to miss
//! in review, impossible to miss as a user.
//!
//! Two things are timed separately, because they fail for different reasons:
//!
//! - **`refresh_processes`** is `sysinfo` reading the operating system. It is
//!   the dominant cost and it varies with how many processes the machine
//!   actually has, so it is measured on the real process table.
//! - **`diff_processes`** is osstat's own work, and it is measured against a
//!   synthetic 500-process table so the number means the same thing on every
//!   runner. This is the half a change to this repository can regress.

// `criterion_group!` expands to a function this crate cannot document, and the
// workspace lints treat a missing doc as an error.
#![allow(missing_docs)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use osstat_core::{ProcessProvider, ProcessRecord, ProcessStatus, diff_processes};
use osstat_platform::SysinfoSource;

/// Builds a process table of `count` processes, shaped like a real one:
/// a handful of roots with the rest hanging off them.
fn synthetic_table(count: u32) -> Vec<ProcessRecord> {
    (0..count)
        .map(|index| ProcessRecord {
            pid: index + 1,
            parent_pid: if index < 8 { None } else { Some(index % 8 + 1) },
            name: format!("process-{index}.exe"),
            exe: Some(format!("C:\\Program Files\\app\\process-{index}.exe")),
            user: Some("tester".to_owned()),
            #[allow(
                clippy::cast_precision_loss,
                reason = "index % 100 fits an f32 mantissa exactly"
            )]
            cpu: (index % 100) as f32 / 10.0,
            memory: u64::from(index) * 1_048_576,
            disk_read_rate: u64::from(index) * 64,
            disk_write_rate: u64::from(index) * 32,
            started_at: 1_700_000_000 + u64::from(index),
            status: ProcessStatus::Running,
        })
        .collect()
}

/// A second tick in which a realistic slice of the table has moved.
fn next_tick(previous: &[ProcessRecord]) -> Vec<ProcessRecord> {
    previous
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let mut next = record.clone();
            // Roughly a fifth of processes move by a visible amount each tick;
            // the rest jitter below display precision and must not be reported.
            if index % 5 == 0 {
                next.cpu += 1.5;
                next.memory += 4 * 1_048_576;
            } else {
                next.cpu += 0.001;
                next.memory += 16;
            }
            next
        })
        .collect()
}

/// Registers both halves of the refresh measurement.
fn benchmarks(c: &mut Criterion) {
    let previous = synthetic_table(500);
    let current = next_tick(&previous);

    c.bench_function("diff_processes/500", |b| {
        b.iter(|| diff_processes(black_box(&previous), black_box(&current)));
    });

    // The real read, on whatever this machine is running. Not deterministic
    // across runners, which is exactly why it is a separate measurement.
    let mut source = SysinfoSource::new();
    c.bench_function("sysinfo_refresh/real", |b| {
        b.iter(|| black_box(source.processes().map(|list| list.len())));
    });
}

criterion_group!(process_refresh, benchmarks);
criterion_main!(process_refresh);
