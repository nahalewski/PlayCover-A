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

const AI_CANONNAME: i32 = 0x2;
const AI_NUMERICHOST: i32 = 0x4;

const AF_UNSPEC: i32 = 0;

const EAI_FAMILY: i32 = 5;
const EAI_NONAME: i32 = 8;
const EAI_SERVICE: i32 = 9;

/// Port for a service name or number string.
fn service_port(service: &str) -> Option<u16> {
    if let Ok(port) = service.parse::<u16>() {
        return Some(port);
    }
    // A few well-known names, which is what /etc/services would provide.
    Some(match service {
        "ftp" => 21,
        "ssh" => 22,
        "telnet" => 23,
        "smtp" => 25,
        "domain" => 53,
        "http" => 80,
        "pop3" => 110,
        "ntp" => 123,
        "imap" => 143,
        "https" => 443,
        _ => return None,
    })
}

fn getaddrinfo(
    env: &mut Environment,
    node_name: ConstPtr<u8>,
    serv_name: ConstPtr<u8>,
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

    let (ai_flags, ai_family, ai_socktype, ai_protocol) = if hints.is_null() {
        (0, AF_UNSPEC, 0, 0)
    } else {
        let hint = env.mem.read(hints);
        (
            hint.ai_flags,
            hint.ai_family,
            hint.ai_socktype,
            hint.ai_protocol,
        )
    };
    if ai_family != AF_UNSPEC && ai_family != AF_INET {
        // Only IPv4 is supported.
        return EAI_FAMILY;
    }
    if node_name.is_null() && serv_name.is_null() {
        return EAI_NONAME;
    }
    if !node_name.is_null() {
        if let Ok(node) = env.mem.cstr_at_utf8(node_name) {
            if crate::ad_blocklist::is_blocked_host(node) {
                log!("Blocked advertising lookup of {:?}", node);
                return EAI_NONAME;
            }
        }
    }

    let port = if serv_name.is_null() {
        0
    } else {
        let service = env.mem.cstr_at_utf8(serv_name).unwrap().to_string();
        match service_port(&service) {
            Some(port) => port,
            None => return EAI_SERVICE,
        }
    };

    // The addresses to return (IPv4 only) and the name for AI_CANONNAME.
    let (addresses, canonical_name): (Vec<[u8; 4]>, Option<String>) = if node_name.is_null() {
        // No host: the loopback address, or the wildcard address for servers.
        let octets = if ai_flags & AI_PASSIVE != 0 {
            [0, 0, 0, 0]
        } else {
            [127, 0, 0, 1]
        };
        (vec![octets], None)
    } else {
        let host = env.mem.cstr_at_utf8(node_name).unwrap().to_string();
        use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
        if let Ok(literal) = host.parse::<Ipv4Addr>() {
            (vec![literal.octets()], Some(host))
        } else if ai_flags & AI_NUMERICHOST != 0 {
            return EAI_NONAME;
        } else {
            let resolved = (host.as_str(), port).to_socket_addrs();
            let mut addresses: Vec<[u8; 4]> = Vec::new();
            if let Ok(found) = resolved {
                for found in found {
                    if let IpAddr::V4(v4) = found.ip() {
                        if !addresses.contains(&v4.octets()) {
                            addresses.push(v4.octets());
                        }
                    }
                }
            }
            if addresses.is_empty() {
                log!("getaddrinfo(\"{}\") => EAI_NONAME (lookup failed)", host);
                return EAI_NONAME;
            }
            log!("getaddrinfo(\"{}\") => {:?}", host, addresses);
            (addresses, Some(host))
        }
    };

    // One entry per address and socket type.
    let socket_types: Vec<(i32, i32)> = match ai_socktype {
        0 => vec![(SOCK_STREAM, IPPROTO_TCP), (SOCK_DGRAM, IPPROTO_UDP)],
        SOCK_STREAM => vec![(SOCK_STREAM, if ai_protocol != 0 { ai_protocol } else { IPPROTO_TCP })],
        SOCK_DGRAM => vec![(SOCK_DGRAM, if ai_protocol != 0 { ai_protocol } else { IPPROTO_UDP })],
        _ => return EAI_NONAME,
    };
    let mut head: MutPtr<addrinfo> = Ptr::null();
    let mut tail: MutPtr<addrinfo> = Ptr::null();
    let mut first = true;
    for octets in addresses {
        for &(socktype, protocol) in &socket_types {
            let sockaddr_ptr = env.mem.alloc_and_write(sockaddr::from_ipv4_parts(octets, port));
            let canonname = match (&canonical_name, first && ai_flags & AI_CANONNAME != 0) {
                (Some(name), true) => env.mem.alloc_and_write_cstr(name.as_bytes()),
                _ => Ptr::null(),
            };
            first = false;
            let entry = env.mem.alloc_and_write(addrinfo {
                ai_flags: 0,
                ai_family: AF_INET,
                ai_socktype: socktype,
                ai_protocol: protocol,
                ai_addrlen: guest_size_of::<sockaddr>(),
                ai_canonname: canonname,
                ai_addr: sockaddr_ptr,
                ai_next: Ptr::null(),
            });
            if head.is_null() {
                head = entry;
            } else {
                let mut previous = env.mem.read(tail);
                previous.ai_next = entry;
                env.mem.write(tail, previous);
            }
            tail = entry;
        }
    }
    env.mem.write(res, head);
    0 // Success
}

fn freeaddrinfo(env: &mut Environment, addrinfo: MutPtr<addrinfo>) {
    let mut node = addrinfo;
    while !node.is_null() {
        let value = env.mem.read(node);
        let next = value.ai_next;
        if !value.ai_addr.is_null() {
            env.mem.free(value.ai_addr.cast());
        }
        if !value.ai_canonname.is_null() {
            env.mem.free(value.ai_canonname.cast());
        }
        env.mem.free(node.cast());
        node = next;
    }
}

fn gethostbyname(env: &mut Environment, name: ConstPtr<u8>) -> MutPtr<hostent> {
    let host = env.mem.cstr_at_utf8(name).unwrap().to_string();
    if !env.options.network_access {
        log!("gethostbyname(\"{}\") => NULL (network access is disabled)", host);
        // TODO: set h_errno
        return Ptr::null();
    }
    if crate::ad_blocklist::is_blocked_host(&host) {
        log!("Blocked advertising lookup of {:?}", host);
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
