//! UDP datagram build/parse, with the IPv4 pseudo-header checksum.

use super::ipv4;
use alloc::vec::Vec;

pub const HDR_LEN: usize = 8;

/// Build a UDP datagram (header + payload) with a correct checksum.
pub fn build(
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let len = (HDR_LEN + payload.len()) as u16;
    let mut dgram = Vec::with_capacity(HDR_LEN + payload.len());
    dgram.extend_from_slice(&src_port.to_be_bytes());
    dgram.extend_from_slice(&dst_port.to_be_bytes());
    dgram.extend_from_slice(&len.to_be_bytes());
    dgram.extend_from_slice(&[0, 0]); // checksum placeholder
    dgram.extend_from_slice(payload);

    // Checksum spans a 12-byte pseudo-header + the UDP datagram.
    let mut buf = Vec::with_capacity(12 + dgram.len());
    buf.extend_from_slice(&src_ip);
    buf.extend_from_slice(&dst_ip);
    buf.push(0);
    buf.push(ipv4::PROTO_UDP);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&dgram);
    let mut csum = ipv4::checksum(&buf);
    if csum == 0 {
        csum = 0xffff; // 0 means "no checksum"; use the equivalent
    }
    dgram[6..8].copy_from_slice(&csum.to_be_bytes());
    dgram
}

/// A parsed UDP datagram: (src_port, dst_port, payload).
pub struct Udp<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub payload: &'a [u8],
}

pub fn parse(data: &[u8]) -> Option<Udp<'_>> {
    if data.len() < HDR_LEN {
        return None;
    }
    let len = u16::from_be_bytes([data[4], data[5]]) as usize;
    let end = len.clamp(HDR_LEN, data.len());
    Some(Udp {
        src_port: u16::from_be_bytes([data[0], data[1]]),
        dst_port: u16::from_be_bytes([data[2], data[3]]),
        payload: &data[HDR_LEN..end],
    })
}
