//! Windows implementations of the osstat capability traits.
//!
//! Milestone-specific work lands here: process control via the Win32 API,
//! socket enumeration, Task Scheduler / `Run` key startup entries, and the
//! `ShellExecuteExW` `runas` elevation helper (ADR-006).

use crate::PlatformId;

pub(crate) const PLATFORM_ID: PlatformId = PlatformId::Windows;
pub(crate) const DISPLAY_NAME: &str = "Windows";

/// Names a disk the way a Windows user would.
///
/// Windows users navigate by drive letter, so the letter leads and the volume
/// label — which is frequently empty — is a parenthetical. `sysinfo` reports the
/// mount point with a trailing separator (`C:\`), which nobody says aloud.
pub(crate) fn disk_display_name(label: &str, mount_point: &str) -> String {
    let letter = mount_point.trim_end_matches(['\\', '/']);
    let letter = if letter.is_empty() {
        mount_point
    } else {
        letter
    };

    match label.trim() {
        "" => letter.to_owned(),
        label => format!("{letter} ({label})"),
    }
}

/// Makes a file runnable by its owner.
///
/// Windows decides what is executable by extension, not by a permission bit, so
/// there is nothing to set. Returning `Ok` rather than an "unsupported" error
/// keeps the caller free of a `cfg` branch.
#[allow(
    clippy::unnecessary_wraps,
    reason = "the Unix implementations genuinely fail on a noexec mount or a denied chmod, and \
              callers need one shape on all three platforms; narrowing this to () would push a \
              cfg branch into osstat-inference, which is what ADR-003 exists to prevent"
)]
pub(crate) fn mark_executable(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

/// Ends a process that has already been identity-checked.
///
/// Windows has no signals. The graceful equivalent is posting `WM_CLOSE` to the
/// process's top-level windows, which is what Task Manager's "End task" does and
/// what gives an application the chance to prompt about unsaved work.
/// `sysinfo::Process::kill` shells out to `taskkill.exe /PID <pid> /F` rather
/// than calling `TerminateProcess` directly, so using it for both steps would
/// make them identical while the UI claimed otherwise.
pub(crate) fn terminate(
    process: &sysinfo::Process,
    mode: osstat_core::TerminationMode,
) -> osstat_core::Result<osstat_core::Termination> {
    match mode {
        osstat_core::TerminationMode::Graceful => {
            if post_close_to_windows_of(process.pid().as_u32()) == 0 {
                Ok(osstat_core::Termination::NoWindowToClose)
            } else {
                Ok(osstat_core::Termination::Signalled)
            }
        }
        osstat_core::TerminationMode::Forceful => {
            if process.kill() {
                Ok(osstat_core::Termination::Signalled)
            } else {
                // `kill()` shells out to `taskkill.exe /PID <pid> /F`, which exits
                // nonzero both when it lacks permission and when the process is
                // simply no longer there — by far the more common case, since five
                // seconds separate the graceful attempt from this one. Re-check
                // existence the same way `Terminator` does before signalling, so
                // "already gone" is not misreported as "not permitted" (ADR-006
                // needs `PermissionDenied` to stay the sole "elevation might help"
                // signal).
                let key = osstat_core::ProcessKey {
                    pid: process.pid().as_u32(),
                    started_at: process.start_time(),
                };
                let mut system = sysinfo::System::new();
                match crate::identity::verify(&mut system, key) {
                    Ok(crate::identity::Identity::Gone)
                    | Err(osstat_core::Error::IdentityMismatch { .. }) => {
                        Ok(osstat_core::Termination::AlreadyGone)
                    }
                    Ok(crate::identity::Identity::Matches) => {
                        Err(osstat_core::Error::PermissionDenied(format!(
                            "cannot end pid {}",
                            process.pid().as_u32()
                        )))
                    }
                    Err(other) => Err(other),
                }
            }
        }
    }
}

/// What `EnumWindows` accumulates as it runs.
struct CloseTarget {
    /// The process whose windows are wanted.
    pid: u32,
    /// How many windows were posted to.
    posted: u32,
}

/// Posts `WM_CLOSE` to every visible top-level window owned by `pid`.
///
/// Returns how many windows were posted to; zero means the process has none.
#[allow(
    unsafe_code,
    reason = "EnumWindows, GetWindowThreadProcessId and PostMessageW are raw \
              Win32 calls with no safe wrapper in the ecosystem. This is the \
              only unsafe in the crate and it neither dereferences a pointer \
              the OS did not give it nor outlives the enumeration."
)]
fn post_close_to_windows_of(pid: u32) -> u32 {
    use windows::Win32::Foundation::{HWND, LPARAM, TRUE, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
    };
    use windows::core::BOOL;

    unsafe extern "system" fn visit(window: HWND, state: LPARAM) -> BOOL {
        // SAFETY: `state` is the &mut CloseTarget handed to EnumWindows below,
        // which outlives the enumeration because EnumWindows is synchronous.
        let target = unsafe { &mut *(state.0 as *mut CloseTarget) };

        let mut owner = 0_u32;
        unsafe { GetWindowThreadProcessId(window, Some(&raw mut owner)) };

        if owner == target.pid && unsafe { IsWindowVisible(window) }.as_bool() {
            // A window that refuses the post is not worth failing over; the
            // forceful step is one confirmation away.
            if unsafe { PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)) }.is_ok() {
                target.posted = target.posted.saturating_add(1);
            }
        }

        TRUE
    }

    let mut target = CloseTarget { pid, posted: 0 };
    let _ = unsafe {
        EnumWindows(
            Some(visit),
            LPARAM(std::ptr::from_mut(&mut target) as isize),
        )
    };

    target.posted
}

/// Microsoft's PCI vendor ID, which marks a software or paravirtual adapter.
const MICROSOFT_VENDOR_ID: u32 = 0x1414;

/// Reads every hardware adapter's memory totals from DXGI.
///
/// Deliberately *not* `IDXGIAdapter3::QueryVideoMemoryInfo`. It looks exactly
/// right and is not: Microsoft documents its `CurrentUsage` as "the
/// application's current video memory usage", making it a budgeting API for a
/// process to manage its own footprint. Reading it would put osstat's own
/// memory on screen under a label claiming to describe the machine. Live usage
/// comes from the performance counters instead.
#[allow(
    unsafe_code,
    reason = "CreateDXGIFactory1, EnumAdapters1 and GetDesc1 are raw Win32 \
              calls with no safe wrapper. Each block below states its own \
              invariant, and nothing borrowed from the OS outlives its call."
)]
pub(crate) fn adapter_memory() -> Vec<crate::AdapterMemory> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIAdapter1, IDXGIFactory1,
    };

    // SAFETY: takes no arguments and returns a COM interface or an error. A
    // machine without DXGI — Windows Server core, some containers — errors
    // here, which is an ordinary empty answer rather than a failure.
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
        return Vec::new();
    };

    let mut adapters = Vec::new();

    for index in 0.. {
        // SAFETY: `factory` is a live COM interface — CreateDXGIFactory1
        // returned Ok above, and it is not released for the loop's duration.
        // The call returns DXGI_ERROR_NOT_FOUND once `index` passes the last
        // adapter, which is this loop's exit condition.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };

        // SAFETY: `adapter` is a valid interface, just returned Ok by
        // EnumAdapters1 above and still owned here. GetDesc1 in windows 0.62
        // returns the description by value rather than writing through a
        // caller pointer, so there is no buffer or lifetime to get wrong.
        let Ok(desc) = (unsafe { IDXGIAdapter1::GetDesc1(&adapter) }) else {
            continue;
        };

        // The Microsoft Basic Render Driver is a CPU rasteriser Windows always
        // enumerates. The GPU probe already excludes it from the device list;
        // DXGI enumerates independently, so the exclusion is made again here.
        // `DXGI_ADAPTER_FLAG_SOFTWARE` alone is not enough. A headless Windows
        // machine with no display driver — a CI runner, a VM — enumerates the
        // Basic Render Driver *without* that flag set, so the flag test passed
        // it straight through. GitHub's windows-latest runner proved it: the
        // guard test below caught a rasteriser being reported as a graphics
        // adapter with memory pools of its own.
        //
        // Vendor `0x1414` is Microsoft, which on a PCI adapter means a software
        // or paravirtual device rather than a card. Neither has video memory to
        // report, which is the only thing this function exists to read.
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0
            || desc.VendorId == MICROSOFT_VENDOR_ID
        {
            continue;
        }

        let end = desc
            .Description
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(desc.Description.len());

        // `phys: 0` because DXGI describes the adapter, not the physical cards
        // behind it. A linked pair — the counters emit `phys_0` and `phys_1`
        // under one LUID — therefore reports only the first card's usage
        // against the pair's shared total, which under-reads. Summing them is
        // the obvious alternative and is not obviously right: the totals come
        // from DXGI's single description, so a sum could exceed them. Left as
        // the conservative read until a machine exists to check it against.
        let luid = crate::gpu_memory::LuidKey {
            high: crate::gpu_memory::luid_high(desc.AdapterLuid.HighPart),
            low: desc.AdapterLuid.LowPart,
            phys: 0,
        };

        adapters.push((
            luid,
            crate::AdapterMemory {
                pci_vendor: desc.VendorId,
                pci_device: desc.DeviceId,
                name: Some(String::from_utf16_lossy(&desc.Description[..end])),
                vram_total: u64::try_from(desc.DedicatedVideoMemory)
                    .ok()
                    .and_then(crate::gpu_memory::non_zero),
                vram_used: None,
                shared_total: u64::try_from(desc.SharedSystemMemory)
                    .ok()
                    .and_then(crate::gpu_memory::non_zero),
                shared_used: None,
                source: osstat_core::GpuSource::Dxgi,
            },
        ));
    }

    let usage = usage_by_adapter();

    adapters
        .into_iter()
        .map(|(luid, mut reading)| {
            if let Some(measured) = usage.get(&luid) {
                // A usage figure with no total cannot be drawn as a meter, so
                // it is taken only where the total came through.
                reading.vram_used = reading.vram_total.and(measured.dedicated);
                reading.shared_used = reading.shared_total.and(measured.shared);
            }
            reading
        })
        .collect()
}

/// Both `GPU Adapter Memory` counters for one adapter.
#[derive(Debug, Default, Clone, Copy)]
struct Usage {
    /// Dedicated video memory in use, in bytes.
    dedicated: Option<u64>,
    /// Borrowed system memory in use, in bytes.
    shared: Option<u64>,
}

/// The counter path for dedicated usage, in locale-invariant English.
const DEDICATED_USAGE: &str = r"\GPU Adapter Memory(*)\Dedicated Usage";
/// The counter path for shared usage, in locale-invariant English.
const SHARED_USAGE: &str = r"\GPU Adapter Memory(*)\Shared Usage";

/// Reads both counters for every adapter, keyed by LUID.
///
/// An empty map means the counter set is unavailable — a container, a stripped
/// install, or a machine whose performance counters are disabled. That is an
/// absent figure, never a zero.
#[allow(
    unsafe_code,
    reason = "the Pdh* family is raw Win32 with no safe wrapper; each block \
              states its own invariant and the query handle is closed on \
              every path out"
)]
fn usage_by_adapter() -> std::collections::HashMap<crate::gpu_memory::LuidKey, Usage> {
    use std::collections::HashMap;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Performance::{PDH_HQUERY, PdhCloseQuery, PdhOpenQueryW};

    let mut query = PDH_HQUERY::default();

    // SAFETY: `query` is a valid out-param — a local `PDH_HQUERY::default()`
    // — and `None` is a documented valid `szDataSource` meaning "the local
    // machine's real-time counters". PdhOpenQueryW writes a handle into
    // `query` only when it returns success, which is checked below.
    if unsafe { PdhOpenQueryW(None, 0, &raw mut query) } != ERROR_SUCCESS.0 {
        return HashMap::new();
    }

    let mut usage: HashMap<crate::gpu_memory::LuidKey, Usage> = HashMap::new();

    for (path, is_dedicated) in [(DEDICATED_USAGE, true), (SHARED_USAGE, false)] {
        for (key, value) in read_counter(query, path) {
            let entry = usage.entry(key).or_default();
            if is_dedicated {
                entry.dedicated = crate::gpu_memory::non_zero(value);
            } else {
                entry.shared = crate::gpu_memory::non_zero(value);
            }
        }
    }

    // SAFETY: `query` was successfully opened by PdhOpenQueryW above, is not
    // shared with any other thread, and is not read again after this call.
    let _ = unsafe { PdhCloseQuery(query) };

    usage
}

/// Adds one wildcard counter to an open query and reads every instance.
///
/// The counter is added by its **English** name. PDH resolves counter paths
/// against the installed display language, so `PdhAddCounterW` with an English
/// path returns `PDH_CSTATUS_NO_OBJECT` on a German or Japanese Windows — a
/// defect that passes every test on an English machine and fails silently for
/// a whole class of users.
#[allow(
    unsafe_code,
    reason = "the Pdh* family is raw Win32 with no safe wrapper; the two-pass \
              buffer protocol is PDH's own documented contract"
)]
fn read_counter(
    query: windows::Win32::System::Performance::PDH_HQUERY,
    path: &str,
) -> Vec<(crate::gpu_memory::LuidKey, u64)> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Performance::{
        PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_LARGE, PDH_HCOUNTER,
        PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    };
    use windows::core::PCWSTR;

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut counter = PDH_HCOUNTER::default();

    // SAFETY: `wide` is NUL-terminated, as PCWSTR requires, and it is not
    // dropped until after the call returns, so the pointer stays valid for
    // the read PDH performs. `counter` is a local out-param.
    if unsafe { PdhAddEnglishCounterW(query, PCWSTR(wide.as_ptr()), 0, &raw mut counter) }
        != ERROR_SUCCESS.0
    {
        return Vec::new();
    }

    // Both counters are raw gauges, not rates, so one collection yields a
    // value — unlike a rate counter, which needs two separated in time.
    // SAFETY: `query` was opened by the caller and still owns `counter`,
    // just added to it above; neither is used from another thread.
    if unsafe { PdhCollectQueryData(query) } != ERROR_SUCCESS.0 {
        return Vec::new();
    }

    let mut bytes = 0_u32;
    let mut items = 0_u32;

    // Sizing pass: with no buffer, PDH reports how large one must be. This
    // "fails" with PDH_MORE_DATA, which is the documented success path here.
    // SAFETY: `bytes` and `items` are owned local out-params PDH writes
    // through; passing `None` for the buffer is the documented way to ask
    // for the required size instead of dereferencing anything.
    let _ = unsafe {
        PdhGetFormattedCounterArrayW(counter, PDH_FMT_LARGE, &raw mut bytes, &raw mut items, None)
    };

    if items == 0 || bytes == 0 {
        return Vec::new();
    }

    // PDH's reported `bytes` covers the fixed-size struct array *and* the
    // counter-instance-name strings it packs in after that array, so it is
    // not simply `items * size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>()`. Rounding
    // up guarantees at least that many bytes of backing storage. Allocating
    // the buffer as `Vec<PDH_FMT_COUNTERVALUE_ITEM_W>` rather than
    // `Vec<u8>` gives it the struct's own alignment instead of `u8`'s, which
    // is what makes writing typed structs into it, and reading them back
    // without a raw cast, sound.
    let item_size = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>().max(1);
    let capacity = (bytes as usize).div_ceil(item_size);
    let mut buffer = vec![PDH_FMT_COUNTERVALUE_ITEM_W::default(); capacity];

    // SAFETY: `buffer` holds `capacity * item_size` bytes, at least the
    // `bytes` PDH itself reported in the sizing pass immediately above, and
    // is aligned to `PDH_FMT_COUNTERVALUE_ITEM_W` rather than to `u8`, so
    // PDH's write — the struct array followed by the packed instance-name
    // strings — lands on valid, correctly aligned memory. It outlives the
    // call.
    let status = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_LARGE,
            &raw mut bytes,
            &raw mut items,
            Some(buffer.as_mut_ptr()),
        )
    };

    if status != ERROR_SUCCESS.0 {
        return Vec::new();
    }

    // Clamped defensively: `items` is reported by the same call that just
    // filled `buffer`, so it should already fit, but nothing prevents
    // indexing past the end of a `Vec` from panicking if it somehow did not.
    let count = (items as usize).min(buffer.len());

    buffer[..count]
        .iter()
        .filter_map(|entry| {
            // SAFETY: `szName` points into `buffer`, which is still alive
            // here, and PDH NUL-terminates every instance name it writes, so
            // reading up to that terminator stays inside the allocation.
            let name = unsafe { entry.szName.to_string() }.ok()?;
            let key = crate::gpu_memory::parse_luid_instance(&name)?;

            // PDH's contract: FmtValue means nothing unless CStatus says the
            // item carries valid data. An instance that disappeared between
            // the sizing pass and the fill pass comes back with a non-valid
            // status and whatever happened to be in the union, which must
            // not be read as a measurement.
            if entry.FmtValue.CStatus != PDH_CSTATUS_VALID_DATA {
                return None;
            }

            // SAFETY: `PDH_FMT_LARGE` was the format requested in both
            // PdhGetFormattedCounterArrayW calls above, which determines that
            // `largeValue` is the union arm PDH populated. Separately, the
            // `CStatus` check immediately above guarantees the value in that arm
            // is a real measurement, not stale data from a vanished instance.
            let value = unsafe { entry.FmtValue.Anonymous.largeValue };
            u64::try_from(value).ok().map(|value| (key, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_drive_letter_loses_its_trailing_separator() {
        assert_eq!(disk_display_name("", "C:\\"), "C:");
    }

    #[test]
    fn a_volume_label_becomes_a_parenthetical() {
        assert_eq!(disk_display_name("Windows", "C:\\"), "C: (Windows)");
    }

    #[test]
    fn a_whitespace_label_is_treated_as_absent() {
        assert_eq!(disk_display_name("   ", "D:\\"), "D:");
    }

    #[test]
    fn a_mount_point_that_is_only_a_separator_survives() {
        assert!(!disk_display_name("", "\\").is_empty());
    }

    #[test]
    #[allow(
        clippy::zombie_processes,
        reason = "the child is what terminate() is exercising; it is ended by the forceful \
                  call below rather than by Child::wait, which is the point of the test"
    )]
    fn a_process_with_no_window_reports_that_rather_than_failing() {
        // A console process has no top-level window, so there is nothing to
        // post WM_CLOSE to. That is information for the UI, not an error: it
        // means offer the forceful step now instead of waiting five seconds
        // for an outcome already known.
        let child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 20 127.0.0.1 > NUL"])
            .spawn()
            .unwrap();
        let pid = sysinfo::Pid::from_u32(child.id());

        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        let process = system.process(pid).unwrap();

        let outcome = terminate(process, osstat_core::TerminationMode::Graceful).unwrap();
        assert_eq!(outcome, osstat_core::Termination::NoWindowToClose);

        // Clean up regardless of what the assertion did.
        let _ = terminate(process, osstat_core::TerminationMode::Forceful);
    }

    #[test]
    fn a_process_already_gone_by_the_forceful_call_is_reported_as_gone_not_denied() {
        // Ends the child directly, bypassing taskkill, so it is already gone by
        // the time `terminate` shells out to check on it — the same situation as
        // a process that exits on its own during the five-second grace period.
        // `kill()` returning false here must not be read as a permission
        // failure, or a vanished process reports as belonging to another user.
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 20 127.0.0.1 > NUL"])
            .spawn()
            .unwrap();
        let pid = sysinfo::Pid::from_u32(child.id());

        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        let process = system.process(pid).unwrap();

        child.kill().unwrap();
        child.wait().unwrap();

        let outcome = terminate(process, osstat_core::TerminationMode::Forceful).unwrap();
        assert_eq!(outcome, osstat_core::Termination::AlreadyGone);
    }

    #[test]
    #[allow(
        clippy::zombie_processes,
        reason = "the child is what terminate() is exercising; it is ended by the forceful \
                  call being tested rather than by Child::wait"
    )]
    fn forceful_termination_ends_a_child() {
        let child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 20 127.0.0.1 > NUL"])
            .spawn()
            .unwrap();
        let pid = sysinfo::Pid::from_u32(child.id());

        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing(),
        );

        let outcome = terminate(
            system.process(pid).unwrap(),
            osstat_core::TerminationMode::Forceful,
        )
        .unwrap();

        assert_eq!(outcome, osstat_core::Termination::Signalled);
    }
}
