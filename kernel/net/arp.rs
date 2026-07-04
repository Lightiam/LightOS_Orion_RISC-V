//! ARP (IPv4-over-Ethernet) packet build/parse. The cache lives in the
//! interface (net/mod.rs); these are pure helpers.

use alloc::vec::Vec;

pub const OP_REQUEST: u16 = 1;
pub const OP_REPLY: u16 = 2;
const HTYPE_ETHERNET: u16 = 1;
const PTYPE_IPV4: u16 = 0x0800;

/// A decoded ARP packet.
pub struct Arp {
    pub op: u16,
    pub sender_mac: [u8; 6],
    pub sender_ip: [u8; 4],
    pub target_ip: [u8; 4],
}

/// Build an ARP request/reply (28-byte payload for an Ethernet frame).
pub fn build(
    op: u16,
    sender_mac: [u8; 6],
    sender_ip: [u8; 4],
    target_mac: [u8; 6],
    target_ip: [u8; 4],
) -> Vec<u8> {
    let mut p = Vec::with_capacity(28);
    p.extend_from_slice(&HTYPE_ETHERNET.to_be_bytes());
    p.extend_from_slice(&PTYPE_IPV4.to_be_bytes());
    p.push(6); // hlen
    p.push(4); // plen
    p.extend_from_slice(&op.to_be_bytes());
    p.extend_from_slice(&sender_mac);
    p.extend_from_slice(&sender_ip);
    p.extend_from_slice(&target_mac);
    p.extend_from_slice(&target_ip);
    p
}

/// Parse an ARP payload (from an Ethernet frame with ethertype 0x0806).
pub fn parse(payload: &[u8]) -> Option<Arp> {
    if payload.len() < 28 {
        return None;
    }
    // Only Ethernet/IPv4 ARP.
    if u16::from_be_bytes([payload[0], payload[1]]) != HTYPE_ETHERNET
        || u16::from_be_bytes([payload[2], payload[3]]) != PTYPE_IPV4
    {
        return None;
    }
    let op = u16::from_be_bytes([payload[6], payload[7]]);
    let mut sender_mac = [0u8; 6];
    let mut sender_ip = [0u8; 4];
    let mut target_ip = [0u8; 4];
    sender_mac.copy_from_slice(&payload[8..14]);
    sender_ip.copy_from_slice(&payload[14..18]);
    target_ip.copy_from_slice(&payload[24..28]);
    Some(Arp {
        op,
        sender_mac,
        sender_ip,
        target_ip,
    })
}
