//! Prints what the GPU probe finds on this machine.
//!
//! Kept in the tree because "osstat does not see my GPU" is the bug report this
//! crate will attract, and asking a reporter to run one command beats asking
//! them to describe their hardware.
//!
//! ```text
//! cargo run -p osstat-llm --example probe-gpus
//! ```

use osstat_core::GpuProvider;
use osstat_llm::HardwareProbe;

fn main() {
    let mut probe = HardwareProbe::new(0);

    let devices = match probe.devices() {
        Ok(devices) => devices,
        Err(error) => {
            eprintln!("probe failed: {error}");
            return;
        }
    };

    if devices.is_empty() {
        println!("No GPUs found. That is a valid result on a headless machine.");
        return;
    }

    let samples = probe.measure().unwrap_or_default();

    for device in &devices {
        let sample = samples.iter().find(|entry| entry.index == device.index);

        // Each pool is labelled from its own source: an NVIDIA card on Windows
        // has its dedicated figure measured by NVML and its shared figure by
        // the performance counters, and neither may borrow the other's
        // credibility.
        let dedicated = pool(
            device.vram_total,
            sample.and_then(|s| s.vram_used),
            device.source.is_measured(),
        );
        let shared = pool(
            device.shared_total,
            sample.and_then(|s| s.shared_used),
            device
                .shared_source
                .is_some_and(osstat_core::GpuSource::is_measured),
        );

        println!(
            "[{}] {} — {:?}, backend {:?}",
            device.index,
            device.name,
            device.kind,
            device.backend.as_deref().unwrap_or("-"),
        );
        println!("      dedicated: {dedicated}");
        println!("      shared:    {shared}");
    }

    if samples.is_empty() {
        println!("\nNo device can be measured live (no NVML-capable GPU).");
    } else {
        println!();
        for sample in &samples {
            println!(
                "[{}] utilisation {:?}%, {:?} C",
                sample.index, sample.utilisation, sample.temperature_c
            );
        }
    }
}

/// Renders one memory pool, or says plainly that it is unknown.
///
/// "unknown" and "0 bytes" are different claims — the probe normalises a
/// reported zero to absent precisely so they cannot be confused — and this
/// output keeps them distinct.
fn pool(total: Option<u64>, used: Option<u64>, measured: bool) -> String {
    let Some(total) = total else {
        return "not reported by this source".to_owned();
    };
    let trust = if measured { "measured" } else { "estimated" };
    match used {
        Some(used) => format!(
            "{} MiB of {} MiB ({trust})",
            used / (1024 * 1024),
            total / (1024 * 1024)
        ),
        None => format!(
            "{} MiB total, usage unknown ({trust})",
            total / (1024 * 1024)
        ),
    }
}
