//! netprobe — exercises the UDP socket API from userspace.
//!
//! Sends a DNS A-query for a hostname to the QEMU SLIRP resolver
//! (10.0.2.3:53) and reads the reply. This proves a LightOS *program*
//! (not just the kernel) can do a real request/response over the
//! network — the primitive an agent needs to call out to a service.
#![no_std]
#![no_main]

use libc_shim::{println, recvfrom, sendto, socket};

const DNS_SERVER: [u8; 4] = [10, 0, 2, 3];
const DNS_PORT: u16 = 53;

/// Build a minimal DNS A-record query for `host` (id 0x4c4f).
fn build_query(host: &str, out: &mut [u8]) -> usize {
    // Header: id, flags (RD), qd=1, an/ns/ar=0.
    out[0..2].copy_from_slice(&0x4c4fu16.to_be_bytes());
    out[2..4].copy_from_slice(&0x0100u16.to_be_bytes());
    out[4..6].copy_from_slice(&1u16.to_be_bytes());
    // 6..12 already zero
    let mut n = 12;
    // QNAME: length-prefixed labels, terminated by a zero byte.
    for label in host.split('.') {
        out[n] = label.len() as u8;
        n += 1;
        out[n..n + label.len()].copy_from_slice(label.as_bytes());
        n += label.len();
    }
    out[n] = 0;
    n += 1;
    out[n..n + 2].copy_from_slice(&1u16.to_be_bytes()); // QTYPE = A
    n += 2;
    out[n..n + 2].copy_from_slice(&1u16.to_be_bytes()); // QCLASS = IN
    n += 2;
    n
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let fd = socket();
    if fd < 0 {
        println!("netprobe: socket() failed: {}", fd);
        return 1;
    }

    let host = "example.com";
    let mut query = [0u8; 64];
    let qlen = build_query(host, &mut query);

    println!(
        "netprobe: DNS query for {} -> {}.{}.{}.{}:{}",
        host, DNS_SERVER[0], DNS_SERVER[1], DNS_SERVER[2], DNS_SERVER[3], DNS_PORT
    );
    let sent = sendto(fd, &query[..qlen], DNS_SERVER, DNS_PORT);
    if sent < 0 {
        println!("netprobe: sendto failed: {}", sent);
        return 1;
    }

    let mut buf = [0u8; 512];
    let (n, src_ip, src_port) = recvfrom(fd, &mut buf);
    if n < 0 {
        println!("netprobe: recvfrom failed: {} (no reply)", n);
        return 1;
    }
    let n = n as usize;

    // Header: check it's a response (QR bit) and read the answer count.
    let is_response = n >= 12 && (buf[2] & 0x80) != 0;
    let ancount = if n >= 12 {
        u16::from_be_bytes([buf[6], buf[7]])
    } else {
        0
    };
    println!(
        "netprobe: reply {} bytes from {}.{}.{}.{}:{} (response={}, answers={})",
        n, src_ip[0], src_ip[1], src_ip[2], src_ip[3], src_port, is_response, ancount
    );

    // If an A record is present, print the resolved IPv4 address.
    if let Some(ip) = first_a_record(&buf[..n]) {
        println!(
            "netprobe: {} resolved to {}.{}.{}.{}",
            host, ip[0], ip[1], ip[2], ip[3]
        );
    }

    if is_response {
        println!("[net] udp round-trip OK");
        0
    } else {
        println!("netprobe: reply was not a DNS response");
        1
    }
}

/// Best-effort scan for the first A-record (type=1, rdlength=4) in a
/// DNS answer section. Skips names by jumping over labels/pointers.
fn first_a_record(msg: &[u8]) -> Option<[u8; 4]> {
    if msg.len() < 12 {
        return None;
    }
    let qd = u16::from_be_bytes([msg[4], msg[5]]) as usize;
    let mut i = 12;
    // Skip the question section.
    for _ in 0..qd {
        i = skip_name(msg, i)?;
        i += 4; // QTYPE + QCLASS
    }
    let an = u16::from_be_bytes([msg[6], msg[7]]) as usize;
    for _ in 0..an {
        i = skip_name(msg, i)?;
        if i + 10 > msg.len() {
            return None;
        }
        let rtype = u16::from_be_bytes([msg[i], msg[i + 1]]);
        let rdlen = u16::from_be_bytes([msg[i + 8], msg[i + 9]]) as usize;
        i += 10;
        if rtype == 1 && rdlen == 4 && i + 4 <= msg.len() {
            return Some([msg[i], msg[i + 1], msg[i + 2], msg[i + 3]]);
        }
        i += rdlen;
    }
    None
}

/// Advance past a DNS name (labels or a compression pointer).
fn skip_name(msg: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let len = *msg.get(i)?;
        if len & 0xc0 == 0xc0 {
            return Some(i + 2); // compression pointer (2 bytes)
        }
        if len == 0 {
            return Some(i + 1);
        }
        i += 1 + len as usize;
    }
}
