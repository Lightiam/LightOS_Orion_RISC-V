//! Ethernet II framing.

use alloc::vec::Vec;

pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const BROADCAST: [u8; 6] = [0xff; 6];

pub const HDR_LEN: usize = 14;

/// Build an Ethernet frame: dst, src, ethertype, then payload.
pub fn build(dst: [u8; 6], src: [u8; 6], ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(HDR_LEN + payload.len());
    f.extend_from_slice(&dst);
    f.extend_from_slice(&src);
    f.extend_from_slice(&ethertype.to_be_bytes());
    f.extend_from_slice(payload);
    f
}

/// A parsed Ethernet frame.
pub struct Frame<'a> {
    pub dst: [u8; 6],
    pub src: [u8; 6],
    pub ethertype: u16,
    pub payload: &'a [u8],
}

pub fn parse(frame: &[u8]) -> Option<Frame<'_>> {
    if frame.len() < HDR_LEN {
        return None;
    }
    let mut dst = [0u8; 6];
    let mut src = [0u8; 6];
    dst.copy_from_slice(&frame[0..6]);
    src.copy_from_slice(&frame[6..12]);
    Some(Frame {
        dst,
        src,
        ethertype: u16::from_be_bytes([frame[12], frame[13]]),
        payload: &frame[HDR_LEN..],
    })
}
