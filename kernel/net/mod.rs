//! Network interface: brings up virtio-net, drives a tiny
//! Ethernet/ARP/IPv4/ICMP stack, and runs a boot-time ping self-test
//! against the QEMU user-mode gateway.
//!
//! Static configuration matches QEMU's SLIRP defaults (10.0.2.0/24,
//! guest 10.0.2.15, gateway 10.0.2.2). DHCP and a userspace socket API
//! are follow-ups; this milestone proves LightOS can get on the wire.

pub mod arp;
pub mod ethernet;
pub mod icmp;
pub mod ipv4;
pub mod socket;
pub mod udp;

use crate::drivers::virtio::net as driver;
use crate::lock::SpinLock;
use crate::uart_println;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, Ordering};

const OUR_IP: [u8; 4] = [10, 0, 2, 15];
const GATEWAY_IP: [u8; 4] = [10, 0, 2, 2];
const ARP_CACHE_LEN: usize = 8;

/// Monotonic IP identification field.
static IP_IDENT: AtomicU16 = AtomicU16::new(1);

struct Iface {
    mac: [u8; 6],
    ip: [u8; 4],
    gateway: [u8; 4],
    arp_cache: [([u8; 4], [u8; 6]); ARP_CACHE_LEN],
    arp_len: usize,
    gateway_mac: Option<[u8; 6]>,
    ping_replies: usize,
}

/// Interface state. Lock invariant: never held across a `driver::send`
/// or `driver::poll_rx` call (those take the device lock).
static IFACE: SpinLock<Option<Iface>> = SpinLock::new(None);

impl Iface {
    fn arp_insert(&mut self, ip: [u8; 4], mac: [u8; 6]) {
        for e in self.arp_cache[..self.arp_len].iter_mut() {
            if e.0 == ip {
                e.1 = mac;
                return;
            }
        }
        if self.arp_len < ARP_CACHE_LEN {
            self.arp_cache[self.arp_len] = (ip, mac);
            self.arp_len += 1;
        }
    }
}

/// Bring up the interface. Returns false if there is no network device.
pub fn init() -> bool {
    if !driver::init() {
        uart_println!("net: no interface (run QEMU with -device virtio-net-device)");
        return false;
    }
    let Some(mac) = driver::mac() else {
        return false;
    };
    *IFACE.lock() = Some(Iface {
        mac,
        ip: OUR_IP,
        gateway: GATEWAY_IP,
        arp_cache: [([0; 4], [0; 6]); ARP_CACHE_LEN],
        arp_len: 0,
        gateway_mac: None,
        ping_replies: 0,
    });
    uart_println!(
        "net: interface up {}.{}.{}.{}/24 gw {}.{}.{}.{}",
        OUR_IP[0],
        OUR_IP[1],
        OUR_IP[2],
        OUR_IP[3],
        GATEWAY_IP[0],
        GATEWAY_IP[1],
        GATEWAY_IP[2],
        GATEWAY_IP[3],
    );
    true
}

/// Drain received frames and answer ARP requests / ICMP echoes.
pub fn poll() {
    let mut frames: Vec<Vec<u8>> = Vec::new();
    driver::poll_rx(|f| frames.push(f.to_vec()));
    for f in frames {
        if let Some(reply) = handle_frame(&f) {
            let _ = driver::send(&reply);
        }
    }
}

/// Process one received frame; returns an Ethernet frame to transmit
/// in response (ARP reply / ICMP echo reply), if any.
fn handle_frame(frame: &[u8]) -> Option<Vec<u8>> {
    let eth = ethernet::parse(frame)?;
    let mut guard = IFACE.lock();
    let iface = guard.as_mut()?;

    match eth.ethertype {
        ethernet::ETHERTYPE_ARP => {
            let a = arp::parse(eth.payload)?;
            iface.arp_insert(a.sender_ip, a.sender_mac);
            match a.op {
                arp::OP_REQUEST if a.target_ip == iface.ip => {
                    let reply = arp::build(
                        arp::OP_REPLY,
                        iface.mac,
                        iface.ip,
                        a.sender_mac,
                        a.sender_ip,
                    );
                    Some(ethernet::build(
                        a.sender_mac,
                        iface.mac,
                        ethernet::ETHERTYPE_ARP,
                        &reply,
                    ))
                }
                arp::OP_REPLY => {
                    if a.sender_ip == iface.gateway {
                        iface.gateway_mac = Some(a.sender_mac);
                    }
                    None
                }
                _ => None,
            }
        }
        ethernet::ETHERTYPE_IPV4 => {
            let pkt = ipv4::parse(eth.payload)?;
            if pkt.dst != iface.ip {
                return None;
            }
            match pkt.proto {
                ipv4::PROTO_ICMP => {
                    let msg = icmp::parse(pkt.payload)?;
                    match msg.msg_type {
                        icmp::TYPE_ECHO_REQUEST => {
                            // Reply, swapping src/dst.
                            let reply =
                                icmp::build(icmp::TYPE_ECHO_REPLY, msg.id, msg.seq, msg.payload);
                            let ident = IP_IDENT.fetch_add(1, Ordering::Relaxed);
                            let ip =
                                ipv4::build(iface.ip, pkt.src, ipv4::PROTO_ICMP, &reply, ident);
                            Some(ethernet::build(
                                eth.src,
                                iface.mac,
                                ethernet::ETHERTYPE_IPV4,
                                &ip,
                            ))
                        }
                        icmp::TYPE_ECHO_REPLY => {
                            if pkt.src == iface.gateway {
                                iface.ping_replies += 1;
                            }
                            None
                        }
                        _ => None,
                    }
                }
                ipv4::PROTO_UDP => {
                    if let Some(d) = udp::parse(pkt.payload) {
                        socket::deliver(d.dst_port, pkt.src, d.src_port, d.payload);
                    }
                    None
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Send an ARP request for `target` (broadcast).
fn send_arp_request(target: [u8; 4]) {
    let (mac, ip) = {
        let guard = IFACE.lock();
        let Some(iface) = guard.as_ref() else {
            return;
        };
        (iface.mac, iface.ip)
    };
    let req = arp::build(arp::OP_REQUEST, mac, ip, [0; 6], target);
    let frame = ethernet::build(ethernet::BROADCAST, mac, ethernet::ETHERTYPE_ARP, &req);
    let _ = driver::send(&frame);
}

/// Send one ICMP echo request to `target` (must be ARP-resolved).
fn send_ping(target: [u8; 4], target_mac: [u8; 6], seq: u16) {
    let (mac, ip) = {
        let guard = IFACE.lock();
        let Some(iface) = guard.as_ref() else {
            return;
        };
        (iface.mac, iface.ip)
    };
    let echo = icmp::build(icmp::TYPE_ECHO_REQUEST, 0x4c4f, seq, b"lightos-ping");
    let ident = IP_IDENT.fetch_add(1, Ordering::Relaxed);
    let pkt = ipv4::build(ip, target, ipv4::PROTO_ICMP, &echo, ident);
    let frame = ethernet::build(target_mac, mac, ethernet::ETHERTYPE_IPV4, &pkt);
    let _ = driver::send(&frame);
}

/// Our configured IPv4 address, if the interface is up.
pub fn our_ip() -> Option<[u8; 4]> {
    IFACE.lock().as_ref().map(|i| i.ip)
}

/// Send a UDP datagram to `dst_ip:dst_port` from `src_port`. Routes
/// everything via the (ARP-resolved) gateway, which matches QEMU
/// SLIRP. Returns an error if the interface isn't up or the gateway
/// MAC isn't known yet.
pub fn send_udp(
    dst_ip: [u8; 4],
    dst_port: u16,
    src_port: u16,
    payload: &[u8],
) -> Result<(), &'static str> {
    let (mac, ip, gw_mac) = {
        let guard = IFACE.lock();
        let iface = guard.as_ref().ok_or("net: interface down")?;
        (iface.mac, iface.ip, iface.gateway_mac)
    };
    let gw_mac = gw_mac.ok_or("net: gateway not resolved")?;
    let dgram = udp::build(ip, dst_ip, src_port, dst_port, payload);
    let ident = IP_IDENT.fetch_add(1, Ordering::Relaxed);
    let pkt = ipv4::build(ip, dst_ip, ipv4::PROTO_UDP, &dgram, ident);
    let frame = ethernet::build(gw_mac, mac, ethernet::ETHERTYPE_IPV4, &pkt);
    driver::send(&frame)
}

/// Spin-poll the RX ring up to a budget, returning early when `done`
/// reports success.
fn poll_until(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..2000 {
        poll();
        if done() {
            return true;
        }
        for _ in 0..2000 {
            core::hint::spin_loop();
        }
    }
    done()
}

fn gateway_mac() -> Option<[u8; 6]> {
    IFACE.lock().as_ref().and_then(|i| i.gateway_mac)
}

fn ping_replies() -> usize {
    IFACE.lock().as_ref().map_or(0, |i| i.ping_replies)
}

/// Boot-time self-test: ARP-resolve the gateway, then ping it.
pub fn self_test() {
    // 1. Resolve the gateway MAC via ARP.
    send_arp_request(GATEWAY_IP);
    if !poll_until(|| gateway_mac().is_some()) {
        uart_println!("net: gateway ARP timed out");
        return;
    }
    let gw_mac = gateway_mac().unwrap();
    uart_println!(
        "net: gateway {}.{}.{}.{} is at {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        GATEWAY_IP[0],
        GATEWAY_IP[1],
        GATEWAY_IP[2],
        GATEWAY_IP[3],
        gw_mac[0],
        gw_mac[1],
        gw_mac[2],
        gw_mac[3],
        gw_mac[4],
        gw_mac[5],
    );

    // 2. Ping the gateway.
    let before = ping_replies();
    send_ping(GATEWAY_IP, gw_mac, 1);
    if poll_until(|| ping_replies() > before) {
        uart_println!(
            "net: ping reply from {}.{}.{}.{}",
            GATEWAY_IP[0],
            GATEWAY_IP[1],
            GATEWAY_IP[2],
            GATEWAY_IP[3],
        );
        uart_println!("[net] milestone: ARP + ICMP over virtio-net OK");
    } else {
        uart_println!("net: ping timed out");
    }
}
