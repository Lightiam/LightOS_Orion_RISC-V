//! UDP socket table. Datagrams received by the interface are queued
//! here per bound local port; `recvfrom` drains them.

use crate::lock::SpinLock;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

const MAX_SOCKETS: usize = 32;
/// Cap the per-socket backlog so a chatty peer can't exhaust the heap.
const MAX_BACKLOG: usize = 16;
/// Ephemeral local ports are handed out from here upward.
const EPHEMERAL_BASE: u16 = 49152;

/// One received datagram waiting to be read.
pub struct Datagram {
    pub src_ip: [u8; 4],
    pub src_port: u16,
    pub data: Vec<u8>,
}

struct UdpSocket {
    local_port: u16,
    rx: VecDeque<Datagram>,
}

struct SocketTable {
    slots: [Option<UdpSocket>; MAX_SOCKETS],
    next_ephemeral: u16,
}

static TABLE: SpinLock<SocketTable> = SpinLock::new(SocketTable {
    slots: [const { None }; MAX_SOCKETS],
    next_ephemeral: EPHEMERAL_BASE,
});

/// Allocate a new unbound UDP socket; returns its table index.
pub fn alloc() -> Option<usize> {
    let mut t = TABLE.lock();
    let idx = t.slots.iter().position(|s| s.is_none())?;
    t.slots[idx] = Some(UdpSocket {
        local_port: 0,
        rx: VecDeque::new(),
    });
    Some(idx)
}

/// Free a socket (on close).
pub fn free(idx: usize) {
    if let Some(slot) = TABLE.lock().slots.get_mut(idx) {
        *slot = None;
    }
}

fn port_in_use(t: &SocketTable, port: u16) -> bool {
    t.slots.iter().flatten().any(|s| s.local_port == port)
}

/// Bind a socket to `port` (0 = pick an ephemeral port). Returns the
/// bound port, or None on conflict / bad index.
pub fn bind(idx: usize, port: u16) -> Option<u16> {
    let mut t = TABLE.lock();
    let chosen = if port == 0 {
        // Find a free ephemeral port.
        let mut p = t.next_ephemeral;
        for _ in 0..(u16::MAX - EPHEMERAL_BASE) {
            if !port_in_use(&t, p) {
                break;
            }
            p = if p >= u16::MAX - 1 {
                EPHEMERAL_BASE
            } else {
                p + 1
            };
        }
        t.next_ephemeral = if p >= u16::MAX - 1 {
            EPHEMERAL_BASE
        } else {
            p + 1
        };
        p
    } else {
        if port_in_use(&t, port) {
            return None;
        }
        port
    };
    let sock = t.slots.get_mut(idx)?.as_mut()?;
    sock.local_port = chosen;
    Some(chosen)
}

/// The local port a socket is bound to (0 if unbound).
pub fn local_port(idx: usize) -> u16 {
    TABLE
        .lock()
        .slots
        .get(idx)
        .and_then(|s| s.as_ref())
        .map_or(0, |s| s.local_port)
}

/// Deliver an inbound datagram to whichever socket owns `dst_port`.
pub fn deliver(dst_port: u16, src_ip: [u8; 4], src_port: u16, data: &[u8]) {
    let mut t = TABLE.lock();
    for sock in t.slots.iter_mut().flatten() {
        if sock.local_port == dst_port {
            if sock.rx.len() < MAX_BACKLOG {
                sock.rx.push_back(Datagram {
                    src_ip,
                    src_port,
                    data: data.to_vec(),
                });
            }
            return;
        }
    }
}

/// Pop the next datagram queued on socket `idx`, if any.
pub fn recv(idx: usize) -> Option<Datagram> {
    TABLE.lock().slots.get_mut(idx)?.as_mut()?.rx.pop_front()
}
