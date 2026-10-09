/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `ifaddrs.h` (interface addresses)

use crate::dyld::FunctionExports;
use crate::export_c_func;
use crate::libc::errno::set_errno;
use crate::mem::{ConstPtr, MutPtr, MutVoidPtr};
use crate::Environment;

/// `struct ifaddrs` has 7 pointer-sized fields (28 bytes on 32-bit iOS):
/// `ifa_next`, `ifa_name`, `ifa_flags`, `ifa_addr`, `ifa_netmask`,
/// `ifa_dstaddr`, `ifa_data`.
#[allow(non_camel_case_types)]
struct ifaddrs {}

const IFADDRS_SIZE: usize = 28;

const IFF_UP: u32 = 0x1;
const IFF_BROADCAST: u32 = 0x2;
const IFF_LOOPBACK: u32 = 0x8;
const IFF_RUNNING: u32 = 0x40;
const IFF_MULTICAST: u32 = 0x8000;

const AF_INET: u8 = 2;
const AF_LINK: u8 = 18;
const IFT_ETHER: u8 = 6;

/// The wireless interface touchHLE pretends to have, with the placeholder MAC
/// address that iOS 7+ reports (and that `sysctl` returns, see sysctl.rs).
const MAC_ADDRESS: [u8; 6] = [0x02, 0, 0, 0, 0, 0];

/// This device's IPv4 address on the network, if the host has one: found by
/// asking the OS which local address it would use to reach the internet (no
/// packets are sent).
fn local_ipv4() -> [u8; 4] {
    use std::net::{IpAddr, UdpSocket};
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            socket.connect("8.8.8.8:80")?;
            socket.local_addr()
        })
        .ok()
        .and_then(|address| match address.ip() {
            IpAddr::V4(v4) if !v4.is_unspecified() => Some(v4.octets()),
            _ => None,
        })
        .unwrap_or([192, 168, 1, 2])
}

fn put_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// A `struct sockaddr_in`: length, family, port 0, address.
fn put_sockaddr_in(buf: &mut [u8], offset: usize, octets: [u8; 4]) {
    buf[offset] = 16;
    buf[offset + 1] = AF_INET;
    buf[offset + 4..offset + 8].copy_from_slice(&octets);
}

/// The interface list: `lo0` (IPv4), and `en0` with its link-layer (MAC)
/// address and IPv4 address. It is a single allocation, so `freeifaddrs()`
/// only has to free the head.
fn getifaddrs(env: &mut Environment, ifap: MutPtr<MutPtr<ifaddrs>>) -> i32 {
    set_errno(env, 0);

    // Layout of the block (offsets in bytes).
    const NODES: usize = 0; // 3 x ifaddrs
    const NAME_LO0: usize = 84;
    const NAME_EN0: usize = 88;
    const LO_ADDR: usize = 92;
    const LO_MASK: usize = 108;
    const EN_ADDR: usize = 124;
    const EN_MASK: usize = 140;
    const EN_LINK: usize = 156; // struct sockaddr_dl, 20 bytes
    const TOTAL: usize = 176;

    let ptr: MutVoidPtr = env.mem.alloc(TOTAL as u32);
    let base = ptr.to_bits();
    let mut block = [0u8; TOTAL];

    block[NAME_LO0..NAME_LO0 + 3].copy_from_slice(b"lo0");
    block[NAME_EN0..NAME_EN0 + 3].copy_from_slice(b"en0");
    put_sockaddr_in(&mut block, LO_ADDR, [127, 0, 0, 1]);
    put_sockaddr_in(&mut block, LO_MASK, [255, 0, 0, 0]);
    put_sockaddr_in(&mut block, EN_ADDR, local_ipv4());
    put_sockaddr_in(&mut block, EN_MASK, [255, 255, 255, 0]);
    // struct sockaddr_dl: len, family, index (2), type, nlen, alen, slen,
    // then the name followed by the address.
    block[EN_LINK] = 20;
    block[EN_LINK + 1] = AF_LINK;
    block[EN_LINK + 2] = 1; // sdl_index
    block[EN_LINK + 4] = IFT_ETHER;
    block[EN_LINK + 5] = 3; // sdl_nlen
    block[EN_LINK + 6] = 6; // sdl_alen
    block[EN_LINK + 8..EN_LINK + 11].copy_from_slice(b"en0");
    block[EN_LINK + 11..EN_LINK + 17].copy_from_slice(&MAC_ADDRESS);

    // (name, flags, address, netmask)
    let nodes: [(usize, u32, usize, usize); 3] = [
        (
            NAME_LO0,
            IFF_UP | IFF_LOOPBACK | IFF_RUNNING | IFF_MULTICAST,
            LO_ADDR,
            LO_MASK,
        ),
        (
            NAME_EN0,
            IFF_UP | IFF_BROADCAST | IFF_RUNNING | IFF_MULTICAST,
            EN_LINK,
            0,
        ),
        (
            NAME_EN0,
            IFF_UP | IFF_BROADCAST | IFF_RUNNING | IFF_MULTICAST,
            EN_ADDR,
            EN_MASK,
        ),
    ];
    for (index, &(name, flags, address, netmask)) in nodes.iter().enumerate() {
        let node = NODES + index * IFADDRS_SIZE;
        let next = if index + 1 < nodes.len() {
            base + (node + IFADDRS_SIZE) as u32
        } else {
            0
        };
        put_u32(&mut block, node, next); // ifa_next
        put_u32(&mut block, node + 4, base + name as u32); // ifa_name
        put_u32(&mut block, node + 8, flags); // ifa_flags
        put_u32(&mut block, node + 12, base + address as u32); // ifa_addr
        let netmask = if netmask == 0 { 0 } else { base + netmask as u32 };
        put_u32(&mut block, node + 16, netmask); // ifa_netmask
    }

    env.mem
        .bytes_at_mut(ptr.cast::<u8>(), TOTAL as u32)
        .copy_from_slice(&block);
    env.mem.write(ifap, ptr.cast());
    0
}

fn freeifaddrs(env: &mut Environment, ifp: MutPtr<ifaddrs>) {
    if !ifp.is_null() {
        env.mem.free(ifp.cast());
    }
}

/// Looks up a network interface index by name. touchHLE only pretends to have
/// one interface, `en0`; any other name returns 0 (and `ENXIO`).
fn if_nametoindex(env: &mut Environment, name: ConstPtr<u8>) -> u32 {
    const ENXIO: i32 = 6;
    // touchHLE pretends to have a single interface, `en0` (see sysctl.rs).
    if env.mem.cstr_at_utf8(name) == Ok("en0") {
        return 1;
    }
    set_errno(env, ENXIO);
    0
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(getifaddrs(_)),
    export_c_func!(freeifaddrs(_)),
    export_c_func!(if_nametoindex(_)),
];
