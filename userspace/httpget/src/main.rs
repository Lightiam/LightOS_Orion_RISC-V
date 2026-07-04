//! httpget — exercises the TCP socket API from userspace.
//!
//! Opens a TCP connection to the QEMU host (10.0.2.2, which SLIRP maps
//! to the host loopback), sends an HTTP/1.0 GET, and reads the whole
//! response. This proves a LightOS program can do a full TCP
//! request/response — the transport an agent needs to call an
//! HTTP-based LLM API.
#![no_std]
#![no_main]

use libc_shim::{connect, println, recv, send, tcp_socket};

/// QEMU SLIRP maps the host to 10.0.2.2.
const HOST: [u8; 4] = [10, 0, 2, 2];
const PORT: u16 = 18080;

#[no_mangle]
extern "C" fn main() -> i32 {
    let fd = tcp_socket();
    if fd < 0 {
        println!("httpget: socket() failed: {}", fd);
        return 1;
    }

    println!(
        "httpget: connecting to {}.{}.{}.{}:{}",
        HOST[0], HOST[1], HOST[2], HOST[3], PORT
    );
    let rc = connect(fd, HOST, PORT);
    if rc < 0 {
        println!("httpget: connect failed: {}", rc);
        return 1;
    }
    println!("httpget: connected");

    let request = b"GET / HTTP/1.0\r\nHost: lightos\r\nConnection: close\r\n\r\n";
    let sent = send(fd, request);
    if sent < 0 {
        println!("httpget: send failed: {}", sent);
        return 1;
    }
    println!("httpget: sent {} byte request", sent);

    // Read the full response, scanning for the server's marker.
    let mut total = 0usize;
    let mut printed_status = false;
    let mut found_marker = false;
    let marker = b"LightOS-TCP-OK";
    let mut tail = [0u8; 32]; // sliding window to catch a split marker
    let mut tail_len = 0usize;

    loop {
        let mut buf = [0u8; 512];
        let n = recv(fd, &mut buf);
        if n < 0 {
            println!("httpget: recv failed: {}", n);
            return 1;
        }
        if n == 0 {
            break; // end of stream
        }
        let n = n as usize;
        total += n;

        if !printed_status {
            // Print the first line of the response (the status line).
            let end = buf[..n].iter().position(|&b| b == b'\r').unwrap_or(n);
            if let Ok(line) = core::str::from_utf8(&buf[..end]) {
                println!("httpget: status: {}", line);
            }
            printed_status = true;
        }

        // Marker scan across chunk boundaries via a small tail buffer.
        let mut window = [0u8; 512 + 32];
        window[..tail_len].copy_from_slice(&tail[..tail_len]);
        window[tail_len..tail_len + n].copy_from_slice(&buf[..n]);
        let wlen = tail_len + n;
        if window[..wlen].windows(marker.len()).any(|w| w == marker) {
            found_marker = true;
        }
        let keep = wlen.min(marker.len() - 1);
        tail[..keep].copy_from_slice(&window[wlen - keep..wlen]);
        tail_len = keep;
    }

    println!("httpget: received {} bytes total", total);
    if found_marker {
        println!("[net] tcp http OK");
        0
    } else {
        println!("httpget: response did not contain the expected marker");
        1
    }
}
