//! ICMP echo (ping) build/parse.

use super::ipv4;
use alloc::vec::Vec;

pub const TYPE_ECHO_REQUEST: u8 = 8;
pub const TYPE_ECHO_REPLY: u8 = 0;

/// Build an ICMP message (type/code + id/seq + payload) with checksum.
pub fn build(msg_type: u8, id: u16, seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(8 + payload.len());
    m.push(msg_type);
    m.push(0); // code
    m.extend_from_slice(&[0, 0]); // checksum placeholder
    m.extend_from_slice(&id.to_be_bytes());
    m.extend_from_slice(&seq.to_be_bytes());
    m.extend_from_slice(payload);
    let csum = ipv4::checksum(&m);
    m[2..4].copy_from_slice(&csum.to_be_bytes());
    m
}

/// A parsed ICMP message: (type, id, seq, payload).
pub struct Icmp<'a> {
    pub msg_type: u8,
    pub id: u16,
    pub seq: u16,
    pub payload: &'a [u8],
}

pub fn parse(data: &[u8]) -> Option<Icmp<'_>> {
    if data.len() < 8 {
        return None;
    }
    Some(Icmp {
        msg_type: data[0],
        id: u16::from_be_bytes([data[4], data[5]]),
        seq: u16::from_be_bytes([data[6], data[7]]),
        payload: &data[8..],
    })
}
