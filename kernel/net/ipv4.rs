//! Minimal IPv4: header build/parse and the internet checksum.

use alloc::vec::Vec;

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_UDP: u8 = 17;
pub const HDR_LEN: usize = 20;

/// Internet checksum (RFC 1071): one's-complement sum of 16-bit words.
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Build an IPv4 header + payload for `proto`.
pub fn build(src: [u8; 4], dst: [u8; 4], proto: u8, payload: &[u8], ident: u16) -> Vec<u8> {
    let total = (HDR_LEN + payload.len()) as u16;
    let mut h = [0u8; HDR_LEN];
    h[0] = 0x45; // IPv4, IHL=5
    h[1] = 0x00; // DSCP/ECN
    h[2..4].copy_from_slice(&total.to_be_bytes());
    h[4..6].copy_from_slice(&ident.to_be_bytes());
    h[6..8].copy_from_slice(&0x4000u16.to_be_bytes()); // Don't Fragment
    h[8] = 64; // TTL
    h[9] = proto;
    // h[10..12] checksum = 0 for computation
    h[12..16].copy_from_slice(&src);
    h[16..20].copy_from_slice(&dst);
    let csum = checksum(&h);
    h[10..12].copy_from_slice(&csum.to_be_bytes());

    let mut pkt = Vec::with_capacity(HDR_LEN + payload.len());
    pkt.extend_from_slice(&h);
    pkt.extend_from_slice(payload);
    pkt
}

/// A parsed IPv4 packet: (src, dst, proto, payload).
pub struct Ipv4<'a> {
    pub src: [u8; 4],
    pub dst: [u8; 4],
    pub proto: u8,
    pub payload: &'a [u8],
}

pub fn parse(pkt: &[u8]) -> Option<Ipv4<'_>> {
    if pkt.len() < HDR_LEN || pkt[0] >> 4 != 4 {
        return None;
    }
    let ihl = (pkt[0] & 0x0f) as usize * 4;
    if ihl < HDR_LEN || pkt.len() < ihl {
        return None;
    }
    let total = u16::from_be_bytes([pkt[2], pkt[3]]) as usize;
    let end = total.min(pkt.len());
    if end < ihl {
        return None;
    }
    let mut src = [0u8; 4];
    let mut dst = [0u8; 4];
    src.copy_from_slice(&pkt[12..16]);
    dst.copy_from_slice(&pkt[16..20]);
    Some(Ipv4 {
        src,
        dst,
        proto: pkt[9],
        payload: &pkt[ihl..end],
    })
}
