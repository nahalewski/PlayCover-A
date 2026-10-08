/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `netdb.h`

use crate::dyld::FunctionExports;
use crate::export_c_func;
use crate::libc::sys::socket::{sockaddr, AF_INET, SOCK_DGRAM, SOCK_STREAM};
use crate::mem::{guest_size_of, ConstPtr, MutPtr, Ptr, SafeRead};
use crate::Environment;

const AI_PASSIVE: i32 = 0x1;

pub const IPPROTO_TCP: i32 = 6;
pub const IPPROTO_UDP: i32 = 17;

const EAI_FAIL: i32 = 4;

#[allow(non_camel_case_types)]
pub type socklen_t = u32;

#[allow(non_camel_case_types)]
#[derive(Copy, Clone)]
#[repr(C, packed)]
struct hostent {
    h_name: MutPtr<u8>,
    h_aliases: MutPtr<MutPtr<u8>>,
    h_addrtype: i32,
    h_length: i32,
    h_addr_list: MutPtr<MutPtr<u8>>,
}
unsafe impl SafeRead for hostent {}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
#[allow(non_camel_case_types)]
pub struct addrinfo {
    ai_flags: i32,
    ai_family: i32,
    ai_socktype: i32,
    ai_protocol: i32,
    ai_addrlen: socklen_t,
    ai_canonname: MutPtr<u8>,
    ai_addr: MutPtr<sockaddr>,
    ai_next: MutPtr<addrinfo>,
}
unsafe impl SafeRead for addrinfo {}

fn getaddrinfo(
    env: &mut Environment,
    node_name: MutPtr<u8>,
    serv_name: MutPtr<u8>,
    hints: ConstPtr<addrinfo>,
    res: MutPtr<MutPtr<addrinfo>>,
) -> i32 {
    if !env.options.network_access {
        log_dbg!(
            "Network access is disabled, getaddrinfo({:?}, {:?}, {:?}, {:?}) -> EAI_FAIL",
            node_name,
            serv_name,
            hints,
            res
        );
        return EAI_FAIL;
    }

    assert!(node_name.is_null()); // TODO

    let hint = env.mem.read(hints);
    let ai_flags = hint.ai_flags;
    assert_eq!(ai_flags, AI_PASSIVE);
    let ai_family = hint.ai_family;
    assert_eq!(ai_family, AF_INET);
    assert!(hint.ai_socktype == SOCK_STREAM || hint.ai_socktype == SOCK_DGRAM);
    assert!(
        hint.ai_protocol == IPPROTO_TCP || hint.ai_protocol == IPPROTO_UDP || hint.ai_protocol == 0
    );
    let ai_addrlen = hint.ai_addrlen;
    assert_eq!(ai_addrlen, 0);
    assert!(hint.ai_canonname.is_null());
    assert!(hint.ai_addr.is_null());
    assert!(hint.ai_next.is_null());

    let mut addr_info = hint;
    let port: u16 = env.mem.cstr_at_utf8(serv_name).unwrap().parse().unwrap();
    log_dbg!("getaddrinfo: port {}", port);
    let addr = sockaddr::from_ipv4_parts([0; 4], port);

    let tmp_addr = env.mem.alloc_and_write(addr);
    addr_info.ai_addr = tmp_addr;
    addr_info.ai_addrlen = guest_size_of::<sockaddr>();

    let tmp_addr_info = env.mem.alloc_and_write(addr_info);
    env.mem.write(res, tmp_addr_info);

    0 // Success
}

fn freeaddrinfo(env: &mut Environment, addrinfo: MutPtr<addrinfo>) {
    let addrinfo_val = env.mem.read(addrinfo);
    assert!(addrinfo_val.ai_next.is_null()); // TODO
    let ai_addrlen = addrinfo_val.ai_addrlen;
    assert_eq!(ai_addrlen, guest_size_of::<sockaddr>());
    env.mem.free(addrinfo_val.ai_addr.cast());
    env.mem.free(addrinfo.cast());
}

fn gethostbyname(env: &mut Environment, name: ConstPtr<u8>) -> MutPtr<hostent> {
    let host = env.mem.cstr_at_utf8(name).unwrap().to_string();
    if !env.options.network_access {
        log!("gethostbyname(\"{}\") => NULL (network access is disabled)", host);
        // TODO: set h_errno
        return Ptr::null();
    }
    use std::net::{IpAddr, ToSocketAddrs};
    let ipv4 = (host.as_str(), 0u16)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| {
            addrs.find_map(|a| match a.ip() {
                IpAddr::V4(v4) => Some(v4),
                IpAddr::V6(_) => None,
            })
        });
    let Some(ipv4) = ipv4 else {
        log!("gethostbyname(\"{}\") => NULL (lookup failed)", host);
        // TODO: set h_errno
        return Ptr::null();
    };
    log!("gethostbyname(\"{}\") => {}", host, ipv4);
    // The result is a per-call allocation that is never freed: apps only keep
    // it briefly and it is tiny.
    let name_copy = env.mem.alloc_and_write_cstr(host.as_bytes());
    let addr_bytes: MutPtr<u8> = env.mem.alloc(4).cast();
    for (i, b) in ipv4.octets().iter().enumerate() {
        env.mem.write(addr_bytes + i as u32, *b);
    }
    let addr_list: MutPtr<MutPtr<u8>> = env.mem.alloc(8).cast();
    env.mem.write(addr_list, addr_bytes);
    env.mem.write(addr_list + 1, Ptr::null());
    let aliases: MutPtr<MutPtr<u8>> = env.mem.alloc(4).cast();
    env.mem.write(aliases, Ptr::null());
    env.mem.alloc_and_write(hostent {
        h_name: name_copy,
        h_aliases: aliases,
        h_addrtype: AF_INET,
        h_length: 4,
        h_addr_list: addr_list,
    })
}

fn gethostent(_env: &mut Environment) -> MutPtr<hostent> {
    log!("TODO: gethostent() => NULL");
    // TODO: set h_errno
    Ptr::null()
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(getaddrinfo(_, _, _, _)),
    export_c_func!(freeaddrinfo(_)),
    export_c_func!(gethostbyname(_)),
    export_c_func!(gethostent()),
];
