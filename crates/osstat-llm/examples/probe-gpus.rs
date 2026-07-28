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

    for device in &devices {
        let vram = device.vram_total.map_or_else(
            || "unknown".to_owned(),
            |bytes| format!("{} MiB", bytes / (1024 * 1024)),
        );
        let trust = if device.source.is_measured() {
            "measured"
        } else {
            "estimated / unreported"
        };

        println!(
            "[{}] {} — {:?}, backend {:?}, VRAM {vram} ({trust}, source {:?})",
            device.index,
            device.name,
            device.kind,
            device.backend.as_deref().unwrap_or("-"),
            device.source,
        );
    }

    match probe.measure() {
        Ok(samples) if samples.is_empty() => {
            println!("\nNo device can be measured live (no NVML-capable GPU).");
        }
        Ok(samples) => {
            println!();
            for sample in &samples {
                println!(
                    "[{}] utilisation {:?}%, VRAM used {:?}, {:?} C",
                    sample.index, sample.utilisation, sample.vram_used, sample.temperature_c
                );
            }
        }
        Err(error) => eprintln!("measurement failed: {error}"),
    }
}
