//! mutex.rs — the PORT HARD MUTEX (P1 hard requirement, QQ1 lineage).
//!
//! The law comes from a measured defect, not taste: `peluda_voice` fell back
//! to an EPHEMERAL port when 8766 was taken, so TWO TTS daemon instances ran
//! side by side and the fallback HID the conflict (caddis-voice-organ brief,
//! 2026-08-25 finding). Therefore:
//!
//! - The organ binds its port EXACTLY. Port 0 ("let the OS pick") is a
//!   runtime REFUSAL in [`bind_exclusive`] — a conflict must be loud, never
//!   relocated. (Tests may discover free ports with port 0 themselves; the
//!   organ runtime path never does.)
//! - The mutex IS the kernel's TCP bind. No lockfile, no pid dance: the
//!   listening socket is held by the process or it is not, and the kernel
//!   arbitrates. A second bind of a held port fails.
//! - Release is `Drop` of the listener — releasing the socket releases the
//!   port. Nothing else to clean up, which is the point.
//!
//! At P5 cutover the organ is the SECOND claimant of the old daemon ports
//! by design: the QQ1 ruling (auto-kill at cutover) retires the old Piper
//! instance first, THEN the organ binds. A bind refusal at that moment is
//! the operator-visible "old daemon still alive" signal (QQ4: port-conflict
//! attempts logged AND surfaced), not a reason to pick another port.

use std::net::{TcpListener, ToSocketAddrs};

/// Why an exclusive bind failed. Carries the REQUESTED port — the number is
/// the operator-facing fact ("8766 is held"), not a socket error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortMutexErr {
    /// Another process holds the port. The mutex is doing its job.
    Occupied(u16),
    /// The address could not be bound at all (no such address, permission,
    /// transient exhaustion). Distinct from Occupied: retrying is suspect.
    Unbindable { port: u16, cause: String },
    /// port 0 — the ephemeral fallback that once hid a dual-instance defect.
    /// Refused categorically at the organ runtime boundary.
    EphemeralRefused,
    /// Name resolution / anything else the std layer reports.
    Other { port: u16, cause: String },
}

impl std::fmt::Display for PortMutexErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortMutexErr::Occupied(p) => {
                write!(f, "port {p} is held by another process — the port mutex refused a second claimant")
            }
            PortMutexErr::Unbindable { port, cause } => write!(f, "port {port} unbindable: {cause}"),
            PortMutexErr::EphemeralRefused => write!(
                f,
                "port 0 refused: the organ never relocates to an ephemeral port — a conflict must stay loud"
            ),
            PortMutexErr::Other { port, cause } => write!(f, "port {port}: {cause}"),
        }
    }
}

/// Bind `127.0.0.1:port` and NOTHING else, exclusively. Holding the returned
/// listener IS holding the mutex; drop it to release.
///
/// Loopback-only is deliberate: the organ is a local sovereign organ, and a
/// LAN-reachable health port is attack surface nobody asked for.
pub fn bind_exclusive(port: u16) -> Result<TcpListener, PortMutexErr> {
    if port == 0 {
        return Err(PortMutexErr::EphemeralRefused);
    }
    let addr = ("127.0.0.1", port)
        .to_socket_addrs()
        .map_err(|e| PortMutexErr::Other {
            port,
            cause: e.to_string(),
        })?
        .next()
        .ok_or_else(|| PortMutexErr::Other {
            port,
            cause: "no address resolved".into(),
        })?;
    TcpListener::bind(addr).map_err(|e| {
        let cause = e.to_string();
        match e.kind() {
            std::io::ErrorKind::AddrInUse => PortMutexErr::Occupied(port),
            std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::PermissionDenied => {
                PortMutexErr::Unbindable { port, cause }
            }
            _ => PortMutexErr::Other { port, cause },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A free port, discovered the only honest way (bind 0, read it, drop).
    /// Test-only helper — the organ runtime never takes this path.
    fn free_port() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").expect("ephemeral probe bind");
        let p = l.local_addr().expect("addr").port();
        drop(l);
        p
    }

    #[test]
    fn port_zero_is_categorically_refused() {
        assert_eq!(
            bind_exclusive(0).unwrap_err(),
            PortMutexErr::EphemeralRefused
        );
    }

    #[test]
    fn bind_hold_conflict_release() {
        let port = free_port();
        // There is an inherent race between free_port() and this bind (some
        // other process could grab it); on a dev box that does not happen in
        // practice, and if it ever does the Occupied assertion below catches
        // the misunderstanding loudly rather than vacuously.
        let held = bind_exclusive(port).expect("first bind on a fresh port");
        match bind_exclusive(port) {
            Err(PortMutexErr::Occupied(p)) => assert_eq!(p, port),
            other => panic!("second bind must conflict, got {other:?}"),
        }
        drop(held);
        // Released: the same port binds again. TIME_WAIT does not apply to a
        // closed LISTENING socket's bind, only to its accepted connections.
        let again = bind_exclusive(port).expect("re-bind after release");
        drop(again);
    }

    #[test]
    fn occupied_error_names_the_port() {
        let held = TcpListener::bind("127.0.0.1:0").expect("probe");
        let port = held.local_addr().unwrap().port();
        assert_eq!(
            bind_exclusive(port).unwrap_err(),
            PortMutexErr::Occupied(port)
        );
        drop(held);
    }
}
