//! The `netstat2`-backed implementation of [`SocketProvider`].
//!
//! Unlike [`crate::SysinfoSource`] this holds no state between calls. A socket
//! table is not a rate — there is nothing here to compare against a previous
//! reading — and `netstat2` exposes no handle worth keeping open, so a fresh
//! read is a fresh probe every time.

use std::net::IpAddr;

use netstat2::{
    AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, SocketInfo, TcpState, get_sockets_info,
};
use osstat_core::{Result, SocketProtocol, SocketProvider, SocketRecord, SocketState};

/// Live socket data, read through `netstat2`.
#[derive(Debug, Default)]
pub struct NetstatSource;

impl NetstatSource {
    /// Builds a provider. Construction cannot fail: there is no handle to
    /// open, only a query to run on demand.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl SocketProvider for NetstatSource {
    fn sockets(&mut self) -> Result<Vec<SocketRecord>> {
        let af_flags = AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6;
        let proto_flags = ProtocolFlags::TCP | ProtocolFlags::UDP;

        // A restricted process — a sandboxed build, a CI runner with cut-down
        // permissions — is the ordinary case, not a failure: an empty ports
        // page is a legitimate answer, a crashed one is not. `netstat2`
        // reports that restriction as an `Err` with no further structure to
        // distinguish it from something worse, so it is treated the same way
        // `GpuProvider` treats a machine with no GPU — as nothing found, not
        // as nothing knowable.
        let sockets = get_sockets_info(af_flags, proto_flags).unwrap_or_default();

        Ok(sockets.iter().map(to_record).collect())
    }
}

/// Converts one `netstat2` entry to osstat's own record shape.
fn to_record(info: &SocketInfo) -> SocketRecord {
    let pids = info.associated_pids.clone();

    match &info.protocol_socket_info {
        ProtocolSocketInfo::Tcp(tcp) => {
            let has_peer = has_peer(tcp.remote_addr, tcp.remote_port);
            SocketRecord {
                protocol: SocketProtocol::Tcp,
                local_address: tcp.local_addr.to_string(),
                local_port: tcp.local_port,
                remote_address: has_peer.then(|| tcp.remote_addr.to_string()),
                remote_port: has_peer.then_some(tcp.remote_port),
                state: Some(map_state(tcp.state)),
                pids,
            }
        }
        ProtocolSocketInfo::Udp(udp) => SocketRecord {
            protocol: SocketProtocol::Udp,
            local_address: udp.local_addr.to_string(),
            local_port: udp.local_port,
            remote_address: None,
            remote_port: None,
            state: None,
            pids,
        },
    }
}

/// Whether a remote endpoint is a real peer rather than `netstat2`'s "no
/// peer" placeholder (`0.0.0.0:0` or `[::]:0`), reported for sockets that have
/// none — chiefly ones still in `LISTEN`. Repeating that placeholder in every
/// listening row would be noise, not information.
fn has_peer(addr: IpAddr, port: u16) -> bool {
    port != 0 && !addr.is_unspecified()
}

/// Maps `netstat2`'s TCP state enum onto osstat's.
fn map_state(state: TcpState) -> SocketState {
    match state {
        TcpState::Closed => SocketState::Closed,
        TcpState::Listen => SocketState::Listen,
        TcpState::SynSent => SocketState::SynSent,
        TcpState::SynReceived => SocketState::SynReceived,
        TcpState::Established => SocketState::Established,
        TcpState::FinWait1 => SocketState::FinWait1,
        TcpState::FinWait2 => SocketState::FinWait2,
        TcpState::CloseWait => SocketState::CloseWait,
        TcpState::Closing => SocketState::Closing,
        TcpState::LastAck => SocketState::LastAck,
        TcpState::TimeWait => SocketState::TimeWait,
        TcpState::DeleteTcb => SocketState::DeleteTcb,
        TcpState::Unknown => SocketState::Unknown,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::net::{Ipv4Addr, TcpListener};

    use super::*;

    #[test]
    fn a_listening_socket_with_no_peer_is_not_reported_as_one() {
        assert!(!has_peer(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));
        assert!(has_peer(IpAddr::V4(Ipv4Addr::LOCALHOST), 4242));
    }

    #[test]
    fn a_zero_port_with_a_real_address_is_still_not_a_peer() {
        // Belt and braces: `netstat2` always pairs the unspecified address
        // with port zero, but the check should not depend on that coupling.
        assert!(!has_peer(IpAddr::V4(Ipv4Addr::LOCALHOST), 0));
    }

    #[test]
    fn a_zombie_style_unknown_state_maps_across() {
        assert_eq!(map_state(TcpState::Unknown), SocketState::Unknown);
        assert_eq!(map_state(TcpState::Listen), SocketState::Listen);
    }

    // The roadmap's mandated integration test: bind a real listener, prove
    // the probe finds it under the right PID, then prove closing it frees the
    // port. This does not touch process termination at all — M1's kill flow
    // does not exist yet, and is explicitly not this milestone's to build —
    // so "the port frees" is demonstrated by dropping the listener itself
    // rather than by killing a process.
    #[test]
    fn a_bound_listener_appears_with_its_pid_and_disappears_once_closed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let pid = std::process::id();

        let mut source = NetstatSource::new();
        let sockets = source.sockets().unwrap();

        let found = sockets.iter().find(|socket| {
            socket.protocol == SocketProtocol::Tcp
                && socket.local_port == port
                && socket.pids.contains(&pid)
        });
        let found = found.expect("the bound listener should be visible to the probe");
        assert_eq!(found.state, Some(SocketState::Listen));
        assert!(found.state.is_some_and(SocketState::is_listening));

        drop(listener);

        let sockets = source.sockets().unwrap();
        assert!(
            !sockets
                .iter()
                .any(|socket| socket.protocol == SocketProtocol::Tcp
                    && socket.local_port == port
                    && socket.pids.contains(&pid)),
            "the port should free once the listener is dropped"
        );
    }
}
