//! Minimal client-side TCP.
//!
//! Implements enough of RFC 793 to be an HTTP client over QEMU SLIRP: a
//! 3-way handshake, in-order receive with cumulative ACKs, data send
//! with ack tracking, and FIN teardown. The path is lossless (SLIRP is
//! local and reliable), so retransmission is a light safety net rather
//! than a full RTO/congestion-control implementation.
//!
//! Concurrency: `deliver()` runs under the interface lock (called from
//! the RX path), so it never sends directly — it only mutates the TCB
//! and queues outgoing segments. The syscall side calls `pump()` (with
//! no lock held) to flush that queue via `net::send_tcp`.

use super::ipv4;
use crate::lock::SpinLock;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

const MAX_TCP: usize = 16;
const EPHEMERAL_BASE: u16 = 49152;
/// Advertised receive window / max buffered received bytes.
const RCV_WND: u16 = 8192;
const RX_CAP: usize = 64 * 1024;

// TCP flags.
pub const FIN: u8 = 0x01;
pub const SYN: u8 = 0x02;
pub const RST: u8 = 0x04;
pub const PSH: u8 = 0x08;
pub const ACK: u8 = 0x10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Closed,
    SynSent,
    Established,
    FinWait1,
    CloseWait,
    LastAck,
}

struct Tcb {
    state: State,
    local_port: u16,
    remote_ip: [u8; 4],
    remote_port: u16,
    our_ip: [u8; 4],
    /// Send sequence space.
    snd_una: u32,
    snd_nxt: u32,
    /// Next sequence number expected from the peer.
    rcv_nxt: u32,
    /// In-order received bytes waiting for recv().
    rx: VecDeque<u8>,
    /// Fully-built TCP segments queued for transmission.
    out: VecDeque<Vec<u8>>,
    /// Last unacked (seq, segment) for retransmission.
    retransmit: Option<(u32, Vec<u8>)>,
    /// Pump ticks since the retransmit segment was last (re)sent.
    rtx_ticks: u32,
    peer_fin: bool,
    reset: bool,
}

struct TcpTable {
    slots: [Option<Tcb>; MAX_TCP],
    next_ephemeral: u16,
    iss: u32,
}

static TABLE: SpinLock<TcpTable> = SpinLock::new(TcpTable {
    slots: [const { None }; MAX_TCP],
    next_ephemeral: EPHEMERAL_BASE,
    iss: 0x1000,
});

/// Allocate a new TCB; returns its table index.
pub fn alloc() -> Option<usize> {
    let mut t = TABLE.lock();
    let idx = t.slots.iter().position(|s| s.is_none())?;
    let port = t.next_ephemeral;
    t.next_ephemeral = if port >= u16::MAX - 1 {
        EPHEMERAL_BASE
    } else {
        port + 1
    };
    t.slots[idx] = Some(Tcb {
        state: State::Closed,
        local_port: port,
        remote_ip: [0; 4],
        remote_port: 0,
        our_ip: [0; 4],
        snd_una: 0,
        snd_nxt: 0,
        rcv_nxt: 0,
        rx: VecDeque::new(),
        out: VecDeque::new(),
        retransmit: None,
        rtx_ticks: 0,
        peer_fin: false,
        reset: false,
    });
    Some(idx)
}

pub fn free(idx: usize) {
    if let Some(slot) = TABLE.lock().slots.get_mut(idx) {
        *slot = None;
    }
}

/// Build a TCP segment (header + payload) with the pseudo-header
/// checksum filled in.
#[allow(clippy::too_many_arguments)]
fn build_segment(
    our_ip: [u8; 4],
    remote_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut seg = Vec::with_capacity(20 + payload.len());
    seg.extend_from_slice(&src_port.to_be_bytes());
    seg.extend_from_slice(&dst_port.to_be_bytes());
    seg.extend_from_slice(&seq.to_be_bytes());
    seg.extend_from_slice(&ack.to_be_bytes());
    seg.push(5 << 4); // data offset = 5 words (20 bytes), no options
    seg.push(flags);
    seg.extend_from_slice(&RCV_WND.to_be_bytes());
    seg.extend_from_slice(&[0, 0]); // checksum placeholder
    seg.extend_from_slice(&[0, 0]); // urgent pointer
    seg.extend_from_slice(payload);

    // Checksum over the 12-byte pseudo-header + segment.
    let mut buf = Vec::with_capacity(12 + seg.len());
    buf.extend_from_slice(&our_ip);
    buf.extend_from_slice(&remote_ip);
    buf.push(0);
    buf.push(ipv4::PROTO_TCP);
    buf.extend_from_slice(&(seg.len() as u16).to_be_bytes());
    buf.extend_from_slice(&seg);
    let csum = ipv4::checksum(&buf);
    seg[16..18].copy_from_slice(&csum.to_be_bytes());
    seg
}

/// A parsed TCP segment view.
pub struct Segment<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub payload: &'a [u8],
}

pub fn parse(data: &[u8]) -> Option<Segment<'_>> {
    if data.len() < 20 {
        return None;
    }
    let data_off = (data[12] >> 4) as usize * 4;
    if data_off < 20 || data.len() < data_off {
        return None;
    }
    Some(Segment {
        src_port: u16::from_be_bytes([data[0], data[1]]),
        dst_port: u16::from_be_bytes([data[2], data[3]]),
        seq: u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
        ack: u32::from_be_bytes([data[8], data[9], data[10], data[11]]),
        flags: data[13],
        payload: &data[data_off..],
    })
}

/// Begin an active open (connect). Queues the initial SYN.
pub fn connect_begin(idx: usize, our_ip: [u8; 4], remote_ip: [u8; 4], remote_port: u16) -> bool {
    let mut t = TABLE.lock();
    let iss = {
        t.iss = t.iss.wrapping_add(0x2718);
        t.iss
    };
    let Some(tcb) = t.slots.get_mut(idx).and_then(|s| s.as_mut()) else {
        return false;
    };
    tcb.remote_ip = remote_ip;
    tcb.remote_port = remote_port;
    tcb.our_ip = our_ip;
    tcb.snd_una = iss;
    tcb.snd_nxt = iss.wrapping_add(1);
    tcb.state = State::SynSent;
    let syn = build_segment(
        our_ip,
        remote_ip,
        tcb.local_port,
        remote_port,
        iss,
        0,
        SYN,
        &[],
    );
    tcb.retransmit = Some((iss, syn.clone()));
    tcb.rtx_ticks = 0;
    tcb.out.push_back(syn);
    true
}

/// Queue application data for sending (must be Established).
pub fn send_data(idx: usize, data: &[u8]) -> bool {
    let mut t = TABLE.lock();
    let Some(tcb) = t.slots.get_mut(idx).and_then(|s| s.as_mut()) else {
        return false;
    };
    if tcb.state != State::Established {
        return false;
    }
    let seq = tcb.snd_nxt;
    let seg = build_segment(
        tcb.our_ip,
        tcb.remote_ip,
        tcb.local_port,
        tcb.remote_port,
        seq,
        tcb.rcv_nxt,
        PSH | ACK,
        data,
    );
    tcb.snd_nxt = tcb.snd_nxt.wrapping_add(data.len() as u32);
    tcb.retransmit = Some((seq, seg.clone()));
    tcb.rtx_ticks = 0;
    tcb.out.push_back(seg);
    true
}

/// Queue a FIN to begin closing.
pub fn close_begin(idx: usize) {
    let mut t = TABLE.lock();
    let Some(tcb) = t.slots.get_mut(idx).and_then(|s| s.as_mut()) else {
        return;
    };
    match tcb.state {
        State::Established => tcb.state = State::FinWait1,
        State::CloseWait => tcb.state = State::LastAck,
        _ => return,
    }
    let seq = tcb.snd_nxt;
    let fin = build_segment(
        tcb.our_ip,
        tcb.remote_ip,
        tcb.local_port,
        tcb.remote_port,
        seq,
        tcb.rcv_nxt,
        FIN | ACK,
        &[],
    );
    tcb.snd_nxt = tcb.snd_nxt.wrapping_add(1);
    tcb.out.push_back(fin);
}

/// Queue a pure ACK for the current rcv_nxt.
fn queue_ack(tcb: &mut Tcb) {
    let ack = build_segment(
        tcb.our_ip,
        tcb.remote_ip,
        tcb.local_port,
        tcb.remote_port,
        tcb.snd_nxt,
        tcb.rcv_nxt,
        ACK,
        &[],
    );
    tcb.out.push_back(ack);
}

/// Handle an inbound segment for the matching connection. Runs under
/// the interface lock, so it only mutates state / queues output.
pub fn deliver(our_ip: [u8; 4], remote_ip: [u8; 4], seg: &Segment) {
    let mut t = TABLE.lock();
    let Some(tcb) = t
        .slots
        .iter_mut()
        .flatten()
        .find(|c| c.local_port == seg.dst_port && c.state != State::Closed)
    else {
        return;
    };
    // Only accept segments from our peer.
    if tcb.state != State::SynSent
        && (remote_ip != tcb.remote_ip || seg.src_port != tcb.remote_port)
    {
        return;
    }
    tcb.our_ip = our_ip;

    if seg.flags & RST != 0 {
        tcb.reset = true;
        tcb.state = State::Closed;
        return;
    }

    if seg.flags & ACK != 0 {
        tcb.snd_una = seg.ack;
        if let Some((seq, _)) = tcb.retransmit {
            if seg.ack.wrapping_sub(seq) > 0 && seg.ack != seq {
                tcb.retransmit = None;
            }
        }
    }

    match tcb.state {
        State::SynSent => {
            if seg.flags & SYN != 0 && seg.flags & ACK != 0 {
                tcb.remote_ip = remote_ip;
                tcb.remote_port = seg.src_port;
                tcb.rcv_nxt = seg.seq.wrapping_add(1);
                tcb.state = State::Established;
                tcb.retransmit = None;
                queue_ack(tcb);
            }
        }
        State::Established | State::FinWait1 | State::CloseWait => {
            // In-order data only; anything else just re-ACKs.
            if !seg.payload.is_empty() && seg.seq == tcb.rcv_nxt {
                if tcb.rx.len() + seg.payload.len() <= RX_CAP {
                    tcb.rx.extend(seg.payload.iter().copied());
                    tcb.rcv_nxt = tcb.rcv_nxt.wrapping_add(seg.payload.len() as u32);
                }
                queue_ack(tcb);
            } else if !seg.payload.is_empty() {
                queue_ack(tcb); // out of order: re-ACK what we have
            }
            if seg.flags & FIN != 0 && seg.seq.wrapping_add(seg.payload.len() as u32) == tcb.rcv_nxt
            {
                tcb.rcv_nxt = tcb.rcv_nxt.wrapping_add(1);
                tcb.peer_fin = true;
                if tcb.state == State::Established {
                    tcb.state = State::CloseWait;
                }
                queue_ack(tcb);
            }
        }
        State::LastAck => {
            if seg.flags & ACK != 0 {
                tcb.state = State::Closed;
            }
        }
        State::Closed => {}
    }
}

/// Flush the outgoing queue (and light retransmit) for one socket.
/// Must be called with no lock held; sends via `net::send_tcp`.
pub fn pump(idx: usize) {
    // Collect segments to send while holding the lock, then release
    // before sending (send_tcp takes the interface lock).
    let (remote_ip, segments) = {
        let mut t = TABLE.lock();
        let Some(tcb) = t.slots.get_mut(idx).and_then(|s| s.as_mut()) else {
            return;
        };
        let mut segs: Vec<Vec<u8>> = tcb.out.drain(..).collect();

        // Retransmit the oldest unacked segment periodically.
        if let Some((_, seg)) = &tcb.retransmit {
            tcb.rtx_ticks += 1;
            if tcb.rtx_ticks >= 400 {
                tcb.rtx_ticks = 0;
                segs.push(seg.clone());
            }
        }
        (tcb.remote_ip, segs)
    };
    for seg in segments {
        let _ = crate::net::send_tcp(remote_ip, &seg);
    }
}

/// Drain up to `max` received bytes into `out`; returns count.
pub fn recv_buffered(idx: usize, out: &mut [u8]) -> usize {
    let mut t = TABLE.lock();
    let Some(tcb) = t.slots.get_mut(idx).and_then(|s| s.as_mut()) else {
        return 0;
    };
    let n = out.len().min(tcb.rx.len());
    for slot in out.iter_mut().take(n) {
        *slot = tcb.rx.pop_front().unwrap();
    }
    n
}

/// True once every byte queued for send has been acknowledged.
pub fn send_complete(idx: usize) -> bool {
    TABLE
        .lock()
        .slots
        .get(idx)
        .and_then(|s| s.as_ref())
        .is_none_or(|c| c.snd_una == c.snd_nxt)
}

pub fn state(idx: usize) -> State {
    TABLE
        .lock()
        .slots
        .get(idx)
        .and_then(|s| s.as_ref())
        .map_or(State::Closed, |c| c.state)
}

pub fn was_reset(idx: usize) -> bool {
    TABLE
        .lock()
        .slots
        .get(idx)
        .and_then(|s| s.as_ref())
        .is_some_and(|c| c.reset)
}

/// True once the peer has sent FIN and all its data has been read.
pub fn at_eof(idx: usize) -> bool {
    let t = TABLE.lock();
    t.slots
        .get(idx)
        .and_then(|s| s.as_ref())
        .is_none_or(|c| c.peer_fin && c.rx.is_empty())
}
