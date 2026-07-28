//! Managed state for on-demand socket enumeration.
//!
//! Unlike the process table, sockets are not part of [`crate::sampler`]'s
//! tick: nothing in the roadmap asks the ports page to stream, and adding an
//! unrequested feed would grow the sampler's per-tick cost for a page most
//! sessions never open. A `netstat2` read is cheap enough to do at the moment
//! the page actually asks for it.

use std::sync::Mutex;

use osstat_core::{PortRecord, ProcessRecord, SocketProvider, join_sockets_to_processes};
use osstat_platform::NetstatSource;

/// Tauri-managed handle around the socket probe.
///
/// Wrapped in a `Mutex` only because Tauri state must be `Sync`; the probe
/// itself ([`NetstatSource`]) holds nothing between calls, so the lock is
/// never held longer than one enumeration.
#[derive(Default)]
pub struct PortInspector(Mutex<NetstatSource>);

impl PortInspector {
    /// Every socket currently open, joined to the given process table.
    ///
    /// Takes the process list as a parameter rather than reading it itself,
    /// so the caller decides how fresh it needs to be — in the app, the
    /// sampler's latest tick; in tests, whatever the test constructs.
    #[must_use]
    pub fn list(&self, processes: &[ProcessRecord]) -> Vec<PortRecord> {
        let mut source = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sockets = source.sockets().unwrap_or_default();
        join_sockets_to_processes(&sockets, processes)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn an_inspector_with_no_processes_still_returns_whatever_sockets_exist() {
        let inspector = PortInspector::default();

        // Just proving the plumbing holds together end to end; the platform
        // enumeration itself is covered in `osstat-platform`.
        let rows = inspector.list(&[]);
        assert!(rows.iter().all(|row| row.process_name.is_none()));
    }
}
