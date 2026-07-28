//! Sockets: the flat records a platform reports, and how they join to the
//! process table.
//!
//! The socket probe and the process probe are two different OS queries with no
//! guaranteed ordering between them, so [`SocketRecord`] carries only what the
//! socket table itself knows — a PID, never a name or a path. Turning that PID
//! into something a user recognises is [`join_sockets_to_processes`], a pure
//! function over two flat lists, for the same reason [`crate::process`] keeps
//! its diffing logic free of any OS call: it is the part worth testing
//! exhaustively on a CI runner with no interesting sockets open at all.
//!
//! # Why `pids` is a `Vec`, not an `Option<u32>`
//!
//! On Linux a forked child inherits its parent's open file descriptors,
//! including a listening socket, so more than one process can legitimately
//! hold the very same one. Collapsing that to a single owner would silently
//! pick one and hide the other. An empty `Vec` means the opposite case: the
//! platform could not attribute the socket to any process, not that no
//! process holds it.
//!
//! # Why the join is by PID alone
//!
//! [`crate::process::ProcessKey`] — PID plus start time — is the identity used
//! everywhere else in this crate, precisely because PIDs are recycled. The
//! socket probe has no start time to offer, only a bare PID, so this join
//! cannot use that identity and accepts the same small risk `sysinfo` accepts
//! internally: a PID recycled in the instant between the socket read and the
//! process read shows the new process's name against the old process's
//! socket. That window is microseconds within a single tick; refusing to
//! attribute a process at all rather than accept it would make the page
//! useless for the sake of a fault nobody would ever observe.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::process::ProcessRecord;

/// Transport-layer protocol a socket was opened on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum SocketProtocol {
    /// Connection-oriented; carries a [`SocketState`].
    Tcp,
    /// Connectionless; has no state to report.
    Udp,
}

/// Where a TCP connection sits in its lifecycle.
///
/// UDP is stateless — see [`SocketRecord::state`] — so this only ever
/// accompanies a [`SocketProtocol::Tcp`] record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum SocketState {
    /// Fully closed.
    Closed,
    /// Accepting new connections.
    Listen,
    /// A connect in progress, SYN sent.
    SynSent,
    /// A connect in progress, SYN received.
    SynReceived,
    /// A connection with data flowing both ways.
    Established,
    /// Local side has begun closing.
    FinWait1,
    /// Local side's close was acknowledged; waiting on the peer.
    FinWait2,
    /// Peer has begun closing; local side has not yet followed.
    CloseWait,
    /// Both sides are closing simultaneously.
    Closing,
    /// Local side is closed and waiting for the peer's final acknowledgement.
    LastAck,
    /// Closed, but held open to catch delayed packets from the old connection.
    TimeWait,
    /// The connection's control block is being torn down.
    DeleteTcb,
    /// The platform reported a state osstat does not model.
    Unknown,
}

impl SocketState {
    /// Whether a socket in this state is accepting new connections.
    ///
    /// This is the highlight rule the roadmap asks for: "what's listening on
    /// port 8080?" is the question this page mainly exists to answer, so a
    /// listening socket needs to stand out from the ephemeral churn of
    /// `TIME_WAIT` and its neighbours.
    #[must_use]
    pub const fn is_listening(self) -> bool {
        matches!(self, Self::Listen)
    }
}

/// One socket, as a platform reports it — before it is joined to a process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SocketRecord {
    /// TCP or UDP.
    pub protocol: SocketProtocol,
    /// The local address this socket is bound to, as text (`"127.0.0.1"`,
    /// `"::1"`).
    pub local_address: String,
    /// The local port this socket is bound to.
    pub local_port: u16,
    /// The remote peer's address, absent for UDP and for a TCP socket with no
    /// peer — chiefly one still in `LISTEN`.
    pub remote_address: Option<String>,
    /// The remote peer's port; present exactly when `remote_address` is.
    pub remote_port: Option<u16>,
    /// `None` for UDP, which has no connection state to report.
    pub state: Option<SocketState>,
    /// Owning process identifiers. See the module docs for why this is a list
    /// rather than a single optional PID.
    pub pids: Vec<u32>,
}

/// One port, joined to the process that holds it — the row the front-end
/// renders.
///
/// A [`SocketRecord`] fans out into one `PortRecord` per entry in
/// [`SocketRecord::pids`], and into exactly one row with `pid: None` when that
/// list is empty, so every socket the probe found is shown exactly once
/// whether it has zero, one, or several owners.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PortRecord {
    /// TCP or UDP.
    pub protocol: SocketProtocol,
    /// The local address this socket is bound to.
    pub local_address: String,
    /// The local port this socket is bound to.
    pub local_port: u16,
    /// The remote peer's address, where there is one.
    pub remote_address: Option<String>,
    /// The remote peer's port, where there is one.
    pub remote_port: Option<u16>,
    /// `None` for UDP.
    pub state: Option<SocketState>,
    /// The owning process, where the platform could attribute one.
    pub pid: Option<u32>,
    /// The owning process's name, where `pid` resolved against the process
    /// table.
    pub process_name: Option<String>,
    /// The owning process's executable path, where it could be read.
    pub process_path: Option<String>,
}

/// Joins raw sockets to the process table by PID.
///
/// See the module docs for why the join is by bare PID rather than the
/// [`crate::process::ProcessKey`] identity used elsewhere in this crate.
#[must_use]
pub fn join_sockets_to_processes(
    sockets: &[SocketRecord],
    processes: &[ProcessRecord],
) -> Vec<PortRecord> {
    let by_pid: HashMap<u32, &ProcessRecord> = processes
        .iter()
        .map(|process| (process.pid, process))
        .collect();

    let to_port_record = |socket: &SocketRecord, pid: Option<u32>| -> PortRecord {
        let owner = pid.and_then(|pid| by_pid.get(&pid).copied());
        PortRecord {
            protocol: socket.protocol,
            local_address: socket.local_address.clone(),
            local_port: socket.local_port,
            remote_address: socket.remote_address.clone(),
            remote_port: socket.remote_port,
            state: socket.state,
            pid,
            process_name: owner.map(|process| process.name.clone()),
            process_path: owner.and_then(|process| process.exe.clone()),
        }
    };

    let mut joined = Vec::with_capacity(sockets.len());

    for socket in sockets {
        if socket.pids.is_empty() {
            joined.push(to_port_record(socket, None));
        } else {
            for &pid in &socket.pids {
                joined.push(to_port_record(socket, Some(pid)));
            }
        }
    }

    joined
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::process::ProcessStatus;

    fn socket(local_port: u16, pids: Vec<u32>) -> SocketRecord {
        SocketRecord {
            protocol: SocketProtocol::Tcp,
            local_address: "127.0.0.1".into(),
            local_port,
            remote_address: None,
            remote_port: None,
            state: Some(SocketState::Listen),
            pids,
        }
    }

    fn process(pid: u32, name: &str) -> ProcessRecord {
        ProcessRecord {
            pid,
            parent_pid: None,
            name: name.into(),
            exe: Some(format!("/usr/bin/{name}")),
            user: None,
            cpu: 0.0,
            memory: 0,
            disk_read_rate: 0,
            disk_write_rate: 0,
            started_at: 0,
            status: ProcessStatus::Running,
        }
    }

    #[test]
    fn only_listening_sockets_are_flagged_for_highlighting() {
        assert!(SocketState::Listen.is_listening());
        for other in [
            SocketState::Established,
            SocketState::TimeWait,
            SocketState::CloseWait,
            SocketState::Closed,
        ] {
            assert!(!other.is_listening());
        }
    }

    #[test]
    fn a_socket_with_one_owner_resolves_its_name_and_path() {
        let sockets = vec![socket(8080, vec![42])];
        let processes = vec![process(42, "web-server")];

        let joined = join_sockets_to_processes(&sockets, &processes);

        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].pid, Some(42));
        assert_eq!(joined[0].process_name.as_deref(), Some("web-server"));
        assert_eq!(
            joined[0].process_path.as_deref(),
            Some("/usr/bin/web-server")
        );
    }

    #[test]
    fn a_socket_with_no_attributed_process_yields_one_row_with_no_pid() {
        let sockets = vec![socket(53, Vec::new())];

        let joined = join_sockets_to_processes(&sockets, &[]);

        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].pid, None);
        assert_eq!(joined[0].process_name, None);
    }

    #[test]
    fn a_socket_shared_by_a_fork_fans_out_one_row_per_owner() {
        // Linux lets a forked child inherit its parent's listening socket, so
        // the same socket can legitimately have two owners.
        let sockets = vec![socket(9000, vec![1, 2])];
        let processes = vec![process(1, "parent"), process(2, "child")];

        let joined = join_sockets_to_processes(&sockets, &processes);

        assert_eq!(joined.len(), 2);
        let names: Vec<_> = joined.iter().map(|row| row.process_name.clone()).collect();
        assert_eq!(
            names,
            vec![Some("parent".to_owned()), Some("child".to_owned())]
        );
    }

    #[test]
    fn a_pid_the_process_table_does_not_know_still_produces_a_row() {
        // A process that exited between the socket read and the process read.
        // The row must still show the PID, just with nothing to name it.
        let sockets = vec![socket(443, vec![999])];

        let joined = join_sockets_to_processes(&sockets, &[]);

        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].pid, Some(999));
        assert_eq!(joined[0].process_name, None);
    }

    #[test]
    fn records_serialise_with_camel_case_keys() {
        let json = serde_json::to_value(socket(80, vec![1])).unwrap();
        let object = json.as_object().unwrap();

        assert!(object.contains_key("localAddress"));
        assert!(object.contains_key("localPort"));
    }
}
