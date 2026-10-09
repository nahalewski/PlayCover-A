/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `sys/socket.h` (Sockets)
//!
//! We currently support blocking TCP and UDP guest sockets on IPv4 addresses.
//!
//! Because fine grain control is needed, those are implemented as
//! _non-blocking_ host sockets. Moreover, app usage of select() is
//! (optimistically) assumed to check for data readiness before calling
//! any of blocking functions.
//! (Check related functions for more details and remediation.)
//!
//! Other note: Rust std::net APIs are "too high level" sometimes,
//! thus some workarounds need to be implemented.
//! (e.g. [TcpListener] does both bind() and listen() on a call
//! to [TcpListener::bind])
//!
//! Useful resources:
//! - [Beej's Guide to Network Programming](https://beej.us/guide/bgnet/html/index-wide.html)

use crate::dyld::{export_c_func, FunctionExports};
use crate::libc::errno::{
    set_errno, EAGAIN, EBADF, ECONNABORTED, ECONNREFUSED, ECONNRESET, EHOSTUNREACH, EINTR, EINVAL,
    EIO, EISCONN, ENETUNREACH, ENOTCONN, ENOTTY, EPIPE, EPROTONOSUPPORT, ETIMEDOUT,
};
use crate::libc::posix_io::{close, find_or_create_socket, is_socket, FileDescriptor};
use crate::libc::time::timeval;
use crate::mem::{
    guest_size_of, ConstPtr, ConstVoidPtr, GuestUSize, MutPtr, MutVoidPtr, Ptr, SafeRead,
};
use crate::Environment;

use crate::abi::DotDotDot;
use crate::libc::netdb::{socklen_t, IPPROTO_TCP, IPPROTO_UDP};
use std::collections::{HashMap, HashSet};
use std::io;
use std::io::{Read, Write};
use std::net::{SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::time::Duration;

pub const AF_INET: i32 = 2;
pub const SOCK_STREAM: i32 = 1;
pub const SOCK_DGRAM: i32 = 2;

const SOL_SOCKET: i32 = 0xffff;
const SO_DEBUG: i32 = 0x1;
const SO_REUSEADDR: i32 = 0x4;
const SO_BROADCAST: i32 = 0x20;
const SO_ERROR: i32 = 0x1007;

#[allow(non_camel_case_types)]
pub type sa_family_t = u8;

const FD_SETSIZE: i32 = 1024;

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
#[allow(non_camel_case_types)]
pub struct sockaddr {
    sa_len: u8,
    sa_family: sa_family_t,
    sa_data: [u8; 14],
}
unsafe impl SafeRead for sockaddr {}
impl sockaddr {
    /// Makes an IPv4 sockaddr from 4 bytes for ip and a port.
    ///
    /// Port is expected to be native endian and
    /// will be converted to big endian internally.
    pub fn from_ipv4_parts(octets: [u8; 4], port: u16) -> Self {
        let mut addr = sockaddr {
            sa_len: 16,
            sa_family: AF_INET as u8,
            sa_data: [0; 14],
        };
        addr.sa_data[0..2].copy_from_slice(&port.to_be_bytes());
        addr.sa_data[2..6].copy_from_slice(&octets);
        addr
    }
    /// Returns 4 bytes for ip and a port.
    ///
    /// Port is returned in the native endian format.
    fn to_ipv4_parts(self) -> ([u8; 4], u16) {
        assert!(self.sa_len == 16 || self.sa_len == 0);
        assert_eq!(self.sa_family, AF_INET as u8);
        let port = u16::from_be_bytes([self.sa_data[0], self.sa_data[1]]);
        let ip = [
            self.sa_data[2],
            self.sa_data[3],
            self.sa_data[4],
            self.sa_data[5],
        ];
        (ip, port)
    }
    fn from_sockaddr_v4(addr: &SocketAddr) -> Self {
        // Only IPV4 for the moment
        assert!(addr.is_ipv4());
        let SocketAddr::V4(ipv4addr) = addr else {
            unreachable!()
        };
        sockaddr::from_ipv4_parts(ipv4addr.ip().octets(), ipv4addr.port())
    }
    pub fn to_sockaddr_v4(self) -> SocketAddrV4 {
        let (ip, port) = self.to_ipv4_parts();
        SocketAddrV4::new(ip.into(), port)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
#[allow(non_camel_case_types)]
pub struct fd_set {
    // 32 4-byte ints should be enough for 1024 file descriptors
    fds_bits: [i32; 32],
}
unsafe impl SafeRead for fd_set {}

struct SocketHostObject {
    /// Type of the socket, [SOCK_STREAM] for TCP or [SOCK_DGRAM] for UDP
    type_: i32,
    /// Set of options
    options: HashSet<i32>,
    /// TCP socket which is yet to be connected
    tcp_listener: Option<TcpListener>,
    /// TCP socket which was connected on host, but not (yet) on the guest side
    pending_tcp_stream: Option<TcpStream>,
    /// Already connected TCP socket
    tcp_stream: Option<TcpStream>,
    /// UDP socket
    udp_socket: Option<UdpSocket>,
}

#[derive(Default)]
pub struct State {
    sockets: HashMap<i32, SocketHostObject>,
}
impl State {
    fn get(env: &Environment) -> &Self {
        &env.libc_state.socket
    }
    fn get_mut(env: &mut Environment) -> &mut Self {
        &mut env.libc_state.socket
    }
}

fn socket(env: &mut Environment, domain: i32, type_: i32, protocol: i32) -> FileDescriptor {
    // TODO: handle errno properly
    set_errno(env, 0);

    if !env.options.network_access {
        log_dbg!(
            "Network access is disabled, socket({}, {}, {}) => -1",
            domain,
            type_,
            protocol
        );
        set_errno(env, EPROTONOSUPPORT);
        return -1;
    }

    assert_eq!(domain, AF_INET);
    assert!(type_ == SOCK_STREAM || type_ == SOCK_DGRAM);
    assert!(protocol == IPPROTO_TCP || protocol == IPPROTO_UDP || protocol == 0);

    let fd = find_or_create_socket(env);
    assert!(!State::get(env).sockets.contains_key(&fd));
    let host_object = SocketHostObject {
        type_,
        options: Default::default(),
        tcp_listener: None,
        pending_tcp_stream: None,
        tcp_stream: None,
        udp_socket: None,
    };
    State::get_mut(env).sockets.insert(fd, host_object);

    log_dbg!("socket({}, {}, {}) => {}", domain, type_, protocol, fd);
    fd
}

/// `FIONBIO`: set or clear non-blocking mode. touchHLE's host sockets are
/// always non-blocking (operations that would block fail with `EAGAIN`).
const FIONBIO: u32 = 0x8004667e;
const SO_TYPE: i32 = 0x1008;

fn ioctl(env: &mut Environment, fd: i32, request: u32, _args: DotDotDot) -> i32 {
    set_errno(env, 0);
    if !State::get(env).sockets.contains_key(&fd) {
        set_errno(env, ENOTTY);
        return -1;
    }
    match request {
        FIONBIO => 0,
        _ => {
            log!("TODO: ioctl({} (socket), {:#x?}, ...) => -1", fd, request);
            set_errno(env, ENOTTY);
            -1
        }
    }
}

fn getsockopt(
    env: &mut Environment,
    socket: i32,
    level: i32,
    option_name: i32,
    option_value: MutVoidPtr,
    option_len: MutPtr<socklen_t>,
) -> i32 {
    // TODO: handle errno properly
    set_errno(env, 0);

    log_dbg!(
        "getsockopt({}, {:#x}, {:#x}, {:?}, {:?})",
        socket,
        level,
        option_name,
        option_value,
        option_len
    );

    let Some(host_object) = State::get(env).sockets.get(&socket) else {
        set_errno(env, EBADF);
        return -1;
    };
    let socket_type = host_object.type_;
    // Only SO_ERROR (no pending error) and SO_TYPE carry information; the
    // buffer sizes and other tuning options read as 0.
    let value: i32 = if level == SOL_SOCKET && option_name == SO_TYPE {
        socket_type
    } else {
        0
    };
    if !option_len.is_null() {
        let option_len_val = env.mem.read(option_len);
        if option_len_val >= 4 && !option_value.is_null() {
            env.mem.write(option_value.cast::<i32>(), value);
            env.mem.write(option_len, 4);
        }
    }
    let _ = SO_ERROR;

    0 // Success
}

fn setsockopt(
    env: &mut Environment,
    socket: i32,
    level: i32,
    option_name: i32,
    option_value: ConstVoidPtr,
    option_len: socklen_t,
) -> i32 {
    // TODO: handle errno properly
    set_errno(env, 0);

    log_dbg!(
        "setsockopt({}, {:#x}, {:#x}, {:?}, {})",
        socket,
        level,
        option_name,
        option_value,
        option_len
    );

    if option_name == SO_DEBUG {
        set_errno(env, EINVAL);
        log!(
            "Warning: Ignore setsockopt SO_DEBUG at level {} for socket {} => -1",
            level,
            socket
        );
        return -1;
    }

    let Some(sock) = State::get(env).sockets.get(&socket) else {
        set_errno(env, EBADF);
        return -1;
    };
    let type_ = sock.type_;

    assert!(type_ == SOCK_STREAM || type_ == SOCK_DGRAM);

    if level != SOL_SOCKET || !(option_name == SO_REUSEADDR || option_name == SO_BROADCAST) {
        // Tuning options (SO_NOSIGPIPE, SO_KEEPALIVE, buffer sizes, ...) have
        // no effect on correctness, so they are accepted and ignored.
        log!(
            "setsockopt({}, level {:#x}, option {:#x}) ignored",
            socket,
            level,
            option_name
        );
        return 0;
    }
    // TODO: SO_REUSEADDR is not supported in std::net (and not so portable)

    assert_eq!(option_len, guest_size_of::<i32>());
    let tmp: ConstPtr<i32> = option_value.cast();
    assert_eq!(env.mem.read(tmp), 1);

    let options = &mut State::get_mut(env)
        .sockets
        .get_mut(&socket)
        .unwrap()
        .options;
    options.insert(option_name);

    0 // Success
}

fn getsockname(
    env: &mut Environment,
    socket: i32,
    address: MutPtr<sockaddr>,
    address_len: MutPtr<socklen_t>,
) -> i32 {
    // TODO: handle errno properly
    set_errno(env, 0);

    let Some(socket_host_object) = State::get(env).sockets.get(&socket) else {
        set_errno(env, EBADF);
        return -1;
    };
    let type_ = socket_host_object.type_;
    assert!(type_ == SOCK_STREAM || type_ == SOCK_DGRAM);

    assert!(socket_host_object.tcp_listener.is_none());
    assert!(socket_host_object.pending_tcp_stream.is_none());

    match socket_host_object.type_ {
        SOCK_DGRAM => {
            let udp_socket = socket_host_object.udp_socket.as_ref().unwrap();
            let socket_addr = udp_socket.local_addr().unwrap();
            let local_guest_addr = sockaddr::from_sockaddr_v4(&socket_addr);
            assert_eq!(env.mem.read(address_len), guest_size_of::<sockaddr>());
            env.mem.write(address, local_guest_addr);
        }
        SOCK_STREAM => {
            let local = socket_host_object
                .tcp_stream
                .as_ref()
                .and_then(|stream| stream.local_addr().ok());
            let Some(socket_addr @ SocketAddr::V4(_)) = local else {
                set_errno(env, ENOTCONN);
                return -1;
            };
            env.mem.write(address, sockaddr::from_sockaddr_v4(&socket_addr));
            env.mem.write(address_len, guest_size_of::<sockaddr>());
        }
        _ => unreachable!(),
    }

    0 // Success
}

fn getpeername(
    env: &mut Environment,
    socket: i32,
    address: MutPtr<sockaddr>,
    address_len: MutPtr<socklen_t>,
) -> i32 {
    set_errno(env, 0);
    let Some(socket_host_object) = State::get(env).sockets.get(&socket) else {
        set_errno(env, EBADF);
        return -1;
    };
    let peer = socket_host_object
        .tcp_stream
        .as_ref()
        .and_then(|stream| stream.peer_addr().ok());
    let Some(socket_addr @ SocketAddr::V4(_)) = peer else {
        set_errno(env, ENOTCONN);
        return -1;
    };
    env.mem.write(address, sockaddr::from_sockaddr_v4(&socket_addr));
    env.mem.write(address_len, guest_size_of::<sockaddr>());
    0
}

fn bind(
    env: &mut Environment,
    socket: i32,
    address: ConstPtr<sockaddr>,
    address_len: socklen_t,
) -> i32 {
    // TODO: handle errno properly
    set_errno(env, 0);

    let Some(socket_host_object) = State::get(env).sockets.get(&socket) else {
        set_errno(env, EBADF);
        return -1;
    };
    let type_ = socket_host_object.type_;
    assert!(type_ == SOCK_STREAM || type_ == SOCK_DGRAM);

    assert_eq!(address_len, guest_size_of::<sockaddr>());
    let sockaddr_val = env.mem.read(address);
    log_dbg!(
        "bind({}, {:?} ({:?}), {})",
        socket,
        address,
        sockaddr_val,
        address_len
    );

    let socket_address = sockaddr_val.to_sockaddr_v4();
    let type_str = match type_ {
        SOCK_STREAM => "TCP",
        SOCK_DGRAM => "UDP",
        _ => unreachable!(),
    };
    log_dbg!("bind: {} socket address {:?}", type_str, socket_address);

    // re-borrow
    let socket_host_object = State::get(env).sockets.get(&socket).unwrap();
    match type_ {
        SOCK_STREAM => {
            assert!(socket_host_object.tcp_listener.is_none());
            let host_socket = TcpListener::bind(socket_address).unwrap();
            // We set host socket as non-blocking in order to have
            // more control of how and when it's used
            host_socket.set_nonblocking(true).unwrap();
            // TODO: set options
            State::get_mut(env)
                .sockets
                .get_mut(&socket)
                .unwrap()
                .tcp_listener = Some(host_socket);
        }
        SOCK_DGRAM => {
            assert!(socket_host_object.udp_socket.is_none());
            let host_socket = UdpSocket::bind(socket_address).unwrap();
            // We set host socket as non-blocking in order to have
            // more control of how and when it's used
            host_socket.set_nonblocking(true).unwrap();
            for &option in &socket_host_object.options {
                if option == SO_BROADCAST {
                    host_socket.set_broadcast(true).unwrap();
                }
            }
            State::get_mut(env)
                .sockets
                .get_mut(&socket)
                .unwrap()
                .udp_socket = Some(host_socket);
        }
        _ => unreachable!(),
    }

    0 // Success
}

fn listen(env: &mut Environment, socket: i32, backlog: i32) -> i32 {
    // TODO: handle errno properly
    set_errno(env, 0);

    let Some(socket_host_object) = State::get(env).sockets.get(&socket) else {
        set_errno(env, EBADF);
        return -1;
    };
    let type_ = socket_host_object.type_;
    assert!(type_ == SOCK_STREAM);

    log!(
        "Warning: listen(socket: {}, backlog: {}), ignoring",
        socket,
        backlog
    );
    0 // Success
}

/// Maps a host I/O error to the iOS `errno` value.
fn io_error_to_errno(error: &io::Error) -> i32 {
    match error.kind() {
        io::ErrorKind::ConnectionRefused => ECONNREFUSED,
        io::ErrorKind::ConnectionReset => ECONNRESET,
        io::ErrorKind::ConnectionAborted => ECONNABORTED,
        io::ErrorKind::NotConnected => ENOTCONN,
        io::ErrorKind::BrokenPipe => EPIPE,
        io::ErrorKind::TimedOut => ETIMEDOUT,
        io::ErrorKind::WouldBlock => EAGAIN,
        io::ErrorKind::AddrInUse => 48,
        io::ErrorKind::AddrNotAvailable => 49,
        io::ErrorKind::Interrupted => EINTR,
        _ => match error.raw_os_error() {
            Some(101) => ENETUNREACH,
            Some(110) => ETIMEDOUT,
            Some(113) => EHOSTUNREACH,
            _ => EIO,
        },
    }
}

fn connect(
    env: &mut Environment,
    socket: i32,
    address: ConstPtr<sockaddr>,
    address_len: socklen_t,
) -> i32 {
    set_errno(env, 0);

    let Some(socket_host_object) = State::get(env).sockets.get(&socket) else {
        set_errno(env, EBADF);
        return -1;
    };
    let type_ = socket_host_object.type_;
    assert!(type_ == SOCK_STREAM);
    if socket_host_object.tcp_stream.is_some() {
        set_errno(env, EISCONN);
        return -1;
    }

    if address_len < guest_size_of::<sockaddr>() {
        set_errno(env, EINVAL);
        return -1;
    }
    let sockaddr_val = env.mem.read(address);
    let socket_address = sockaddr_val.to_sockaddr_v4();
    log_dbg!(
        "connect({:?} ({:?}), {}) => {:?}",
        address,
        sockaddr_val,
        address_len,
        socket_address
    );

    // The connection is made right away, so non-blocking apps never see
    // EINPROGRESS. A host connection that can't be made fails with the
    // matching errno instead of ending the emulator.
    let host_stream = match TcpStream::connect_timeout(
        &SocketAddr::V4(socket_address),
        Duration::from_secs(15),
    ) {
        Ok(stream) => stream,
        Err(error) => {
            let errno = io_error_to_errno(&error);
            log!(
                "connect({}) to {} failed: {} => errno {}",
                socket,
                socket_address,
                error,
                errno
            );
            set_errno(env, errno);
            return -1;
        }
    };
    // We set host socket as non-blocking in order to have
    // more control of how and when it's used
    host_stream.set_nonblocking(true).unwrap();
    let _ = host_stream.set_nodelay(true);
    State::get_mut(env)
        .sockets
        .get_mut(&socket)
        .unwrap()
        .tcp_stream = Some(host_stream);

    0 // Success
}

fn select(
    env: &mut Environment,
    n_fds: i32,
    read_fds: MutPtr<fd_set>,
    write_fds: MutPtr<fd_set>,
    error_fds: MutPtr<fd_set>,
    timeout: MutPtr<timeval>,
) -> i32 {
    // select() as a sleep and invalid arguments are handled by select_once().
    let deadline = if timeout.is_null() {
        None
    } else {
        let timeval = env.mem.read(timeout);
        let (tv_sec, tv_usec) = (timeval.tv_sec, timeval.tv_usec);
        if tv_sec == 0 && tv_usec == 0 || n_fds == 0 {
            return select_once(env, n_fds, read_fds, write_fds, error_fds, timeout);
        }
        Some(
            std::time::Instant::now()
                + Duration::from_secs(tv_sec.max(0) as u64)
                + Duration::from_micros(tv_usec.max(0) as u64),
        )
    };

    // Wait until a descriptor is ready or the timeout is over: the sets are
    // restored for every attempt, as select_once() clears the bits of
    // descriptors that are not ready.
    let saved: [Option<fd_set>; 3] = [read_fds, write_fds, error_fds]
        .map(|ptr| if ptr.is_null() { None } else { Some(env.mem.read(ptr)) });
    loop {
        for (ptr, set) in [read_fds, write_fds, error_fds].into_iter().zip(saved) {
            if let Some(set) = set {
                env.mem.write(ptr, set);
            }
        }
        let count = select_once(env, n_fds, read_fds, write_fds, error_fds, timeout);
        if count != 0 || deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            return count;
        }
        env.sleep(Duration::from_millis(1));
    }
}

/// A single non-blocking check, see [select].
fn select_once(
    env: &mut Environment,
    n_fds: i32,
    read_fds: MutPtr<fd_set>,
    write_fds: MutPtr<fd_set>,
    error_fds: MutPtr<fd_set>,
    timeout: MutPtr<timeval>,
) -> i32 {
    // TODO: handle errno properly
    set_errno(env, 0);

    if !(0..=FD_SETSIZE).contains(&n_fds) {
        set_errno(env, EINVAL);
        return -1;
    }

    if n_fds == 0 {
        // Apparently, this is a portable way of using select() as usleep()
        // https://stackoverflow.com/questions/3125645/why-use-select-instead-of-sleep
        // ¯\_(ツ)_/¯
        assert!(read_fds.is_null());
        assert!(write_fds.is_null());
        assert!(error_fds.is_null());
        let timeval = env.mem.read(timeout);
        let duration = Duration::from_secs(timeval.tv_sec.try_into().unwrap())
            + Duration::from_micros(timeval.tv_usec.try_into().unwrap());
        log_dbg!("select() used as sleep for {:?}", duration);
        env.sleep(duration);
        return 0;
    }

    // Waiting for a timeout is done by select().
    let should_block = false;

    let mut count = 0;

    if !read_fds.is_null() {
        let mut read_set = env.mem.read(read_fds);
        log_dbg!("select: read_set before {:?}", read_set);
        count += process_set(env, &mut read_set, n_fds, |env, fd, bits, bit_index| {
            log_dbg!("select: bit set in read_set at fd: {}", fd);
            // Only sockets for now
            assert!(is_socket(env, fd));
            // Clean bit in the set for the current socket
            *bits &= !(1 << bit_index);
            let socket_host_object = State::get(env).sockets.get(&fd).unwrap();
            let type_ = socket_host_object.type_;
            match type_ {
                SOCK_DGRAM => {
                    let udp_socket = socket_host_object.udp_socket.as_ref().unwrap();
                    // Peek just one byte to check if we have some data
                    let mut buf = [0; 1];
                    match udp_socket.peek(&mut buf) {
                        Ok(received) => {
                            log_dbg!("select: Socket {} peeked {} bytes", fd, received);
                            // Set bit back
                            *bits |= 1 << bit_index;
                            true
                        }
                        // On Windows, if we receive more bytes than we peek,
                        // it will error, but it means that there is some data!
                        Err(ref e)
                            if cfg!(target_os = "windows") && e.raw_os_error() == Some(10040) =>
                        {
                            // 10040 code is WSAEMSGSIZE
                            log_dbg!(
                                "[Windows case] select: received {} bytes (at least)",
                                buf.len()
                            );
                            // Set bit back
                            *bits |= 1 << bit_index;
                            true
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                            log_dbg!("select: Socket {} would block on peeking, continue.", fd);
                            assert!(!should_block); // TODO
                            false
                        }
                        Err(e) => {
                            panic!("select: Peek for socket {fd} failed: {e:?}")
                        }
                    }
                }
                SOCK_STREAM => {
                    if socket_host_object.tcp_stream.is_none() {
                        // If we don't have a TCP stream it probably means
                        // that a listener is waiting for connection
                        let listener = socket_host_object.tcp_listener.as_ref().unwrap();
                        // The listener is non-blocking,
                        // so we can try to accept
                        match listener.accept() {
                            Ok((stream, addr)) => {
                                log!("select: New client: {}", addr);
                                // We set host socket as non-blocking in order
                                // to have more control of how and when it's
                                // used
                                stream.set_nonblocking(true).unwrap();
                                // We already accepted the connection on
                                // the host, but we need to postpone new
                                // guest fd creation up until guest calls
                                // accept()
                                assert!(socket_host_object.pending_tcp_stream.is_none());
                                State::get_mut(env)
                                    .sockets
                                    .get_mut(&fd)
                                    .unwrap()
                                    .pending_tcp_stream = Some(stream);
                                // Set bit back
                                *bits |= 1 << bit_index;
                                return true;
                            }
                            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                                // No incoming connection is ready
                                log_dbg!("select: TCP listener for socket {} would block on accepting, continue.", fd);
                                assert!(!should_block); // TODO
                                return false;
                            }
                            Err(e) => {
                                panic!("select: Socket {fd} has error accepting connection: {e}");
                            }
                        }
                    }
                    let stream = socket_host_object.tcp_stream.as_ref().unwrap();
                    // Peek just one byte to check if we have some data
                    let mut buf = [0; 1];
                    match stream.peek(&mut buf) {
                        Ok(received) => {
                            log_dbg!("select: received {} bytes (at least)", received);
                            // Set bit back
                            *bits |= 1 << bit_index;
                            true
                        }
                        // On Windows, if we receive more bytes than we peek,
                        // it will error, but it means that there is some data!
                        Err(ref e)
                            if cfg!(target_os = "windows") && e.raw_os_error() == Some(10040) =>
                        {
                            // 10040 code is WSAEMSGSIZE
                            log_dbg!(
                                "[Windows case] select: received {} bytes (at least)",
                                buf.len()
                            );
                            // Set bit back
                            *bits |= 1 << bit_index;
                            true
                        }
                        // As tested on macOS, this marks socket as readable
                        Err(ref e) if e.kind() == io::ErrorKind::ConnectionReset => {
                            log!("select: Peek for socket {}: ConnectionReset", fd);
                            // Set bit back
                            *bits |= 1 << bit_index;
                            true
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                            log_dbg!(
                                "select: TCP stream for socket {} would block on peeking, continue.",
                                fd
                            );
                            assert!(!should_block); // TODO
                            false
                        }
                        Err(e) => {
                            panic!("select: Peek for socket {fd} failed: {e}")
                        }
                    }
                }
                _ => unimplemented!(),
            }
        });
        log_dbg!("select: read_set after {:?}", read_set);
        env.mem.write(read_fds, read_set);
    }

    if !write_fds.is_null() {
        let mut write_set = env.mem.read(write_fds);
        log_dbg!("select: write_set before {:?}", write_set);
        count += process_set(env, &mut write_set, n_fds, |env, fd, bits, bit_index| {
            log_dbg!("select: bit set in write_set at fd: {}", fd);
            // Only sockets for now
            assert!(is_socket(env, fd));
            // Clean bit in the set for the current socket
            *bits &= !(1 << bit_index);
            let socket_host_object = State::get(env).sockets.get(&fd).unwrap();
            let type_ = socket_host_object.type_;
            match type_ {
                SOCK_STREAM => {
                    assert!(socket_host_object.tcp_listener.is_none());
                    // As we cannot "peek" on write, we just check
                    // if TCP stream exist or not
                    // TODO: find a better way
                    if socket_host_object.tcp_stream.is_some() {
                        // Set bit back
                        *bits |= 1 << bit_index;
                        true
                    } else {
                        assert!(!should_block); // TODO
                        false
                    }
                }
                SOCK_DGRAM => {
                    // As we cannot "peek" on write, we just check
                    // if UDP socket exist or not
                    // TODO: find a better way
                    if socket_host_object.udp_socket.is_some() {
                        // Set bit back
                        *bits |= 1 << bit_index;
                        true
                    } else {
                        assert!(!should_block); // TODO
                        false
                    }
                }
                _ => unimplemented!(),
            }
        });
        log_dbg!("select: write_set after {:?}", write_set);
        env.mem.write(write_fds, write_set);
    }

    if !error_fds.is_null() {
        let mut error_set = env.mem.read(error_fds);
        log_dbg!("select: error_set before {:?}", error_set);
        count += process_set(env, &mut error_set, n_fds, |env, fd, bits, bit_index| {
            log_dbg!("select: bit set in error_set at fd: {}", fd);
            // Only sockets for now
            assert!(is_socket(env, fd));
            // Clean bit in the set for the current socket
            *bits &= !(1 << bit_index);
            let socket_host_object = State::get(env).sockets.get(&fd).unwrap();
            let type_ = socket_host_object.type_;
            match type_ {
                SOCK_STREAM => {
                    assert!(socket_host_object.tcp_listener.is_none());
                    let stream = socket_host_object.tcp_stream.as_ref().unwrap();
                    match stream.take_error() {
                        Ok(None) => {
                            log_dbg!("No error on TCP socket {}", fd);
                            false
                        }
                        Ok(Some(error)) => unimplemented!("TCP socket {} error: {:?}", fd, error),
                        Err(error) => panic!("TCP socket {fd} take_error failed: {error:?}"),
                    }
                }
                SOCK_DGRAM => {
                    todo!()
                }
                _ => unimplemented!(),
            }
        });
        log_dbg!("select: error_set after {:?}", error_set);
        env.mem.write(error_fds, error_set);
    }

    count
}

fn process_set<F: Fn(&mut Environment, FileDescriptor, &mut i32, i32) -> bool>(
    env: &mut Environment,
    set: &mut fd_set,
    n_fds: i32,
    process_bit: F,
) -> i32 {
    let mut fds_bits = set.fds_bits;
    let mut count = 0;
    'outer: for (i, bits) in fds_bits.iter_mut().enumerate() {
        for bit_index in 0..32i32 {
            let fd: FileDescriptor = (i as i32) * 32 + bit_index;
            if fd > n_fds {
                break 'outer;
            }
            if (*bits & (1 << bit_index)) != 0 && process_bit(env, fd, bits, bit_index) {
                count += 1;
            }
        }
    }
    set.fds_bits = fds_bits;
    count
}

fn accept(
    env: &mut Environment,
    socket: i32,
    address: MutPtr<sockaddr>,
    address_len: MutPtr<socklen_t>,
) -> FileDescriptor {
    // TODO: handle errno properly
    set_errno(env, 0);

    let Some(socket_host_object) = State::get(env).sockets.get(&socket) else {
        set_errno(env, EBADF);
        return -1;
    };
    let type_ = socket_host_object.type_;
    assert!(type_ == SOCK_STREAM);

    if let Some(stream) = State::get_mut(env)
        .sockets
        .get_mut(&socket)
        .unwrap()
        .pending_tcp_stream
        .take()
    {
        let addr = stream.peer_addr().unwrap();
        // We have already accepted TCP socket, we now need to
        // let guest know as well!
        let new_fd = find_or_create_socket(env);
        assert!(!State::get(env).sockets.contains_key(&new_fd));
        let host_object = SocketHostObject {
            type_: SOCK_STREAM,
            options: Default::default(),
            tcp_listener: None,
            pending_tcp_stream: None,
            tcp_stream: Some(stream),
            udp_socket: None,
        };
        State::get_mut(env).sockets.insert(new_fd, host_object);
        assert!(!address.is_null());
        let peer_guest_addr = sockaddr::from_sockaddr_v4(&addr);
        env.mem.write(address, peer_guest_addr);
        assert_eq!(guest_size_of::<sockaddr>(), env.mem.read(address_len));
        env.mem.write(address_len, guest_size_of::<sockaddr>());
        return new_fd;
    }

    // re-borrow
    let socket_host_object = State::get(env).sockets.get(&socket).unwrap();
    let listener = socket_host_object.tcp_listener.as_ref().unwrap();
    match listener.accept() {
        Ok((_, addr)) => {
            log!("accept: New client: {}", addr);
            unimplemented!()
        }
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
            // No incoming connection is ready
            // TODO: if this happened, take a deep breath and do:
            // - block guest thread with a new [ThreadBlock] type
            // - poll for data in thread scheduling part
            // - write/read/accept/etc data once it is ready
            // - unblock guest thread
            unimplemented!("accept: TCP listener for socket {} would block on accepting, block current guest thread {}.", socket, env.current_thread)
        }
        Err(e) => {
            panic!("accept: Socket {socket} has error accepting connection: {e}");
        }
    }
}

fn recv(
    env: &mut Environment,
    socket: i32,
    buffer: MutVoidPtr,
    length: GuestUSize,
    flags: i32,
) -> i32 {
    recvfrom(env, socket, buffer, length, flags, Ptr::null(), Ptr::null())
}

fn recvfrom(
    env: &mut Environment,
    socket: i32,
    buffer: MutVoidPtr,
    length: GuestUSize,
    flags: i32,
    address: MutPtr<sockaddr>,
    address_len: MutPtr<socklen_t>,
) -> i32 {
    // TODO: handle errno properly
    set_errno(env, 0);

    log_dbg!(
        "recvfrom({}, {:?}, {}, {}, {:?}, {:?})",
        socket,
        buffer,
        length,
        flags,
        address,
        address_len
    );

    if !State::get(env).sockets.contains_key(&socket) {
        set_errno(env, EBADF);
        log!(
            "Warning: recvfrom({}, ...) failed for unknown socket, returning -1",
            socket
        );
        return -1;
    }

    let type_ = State::get(env).sockets.get(&socket).unwrap().type_;
    assert!(type_ == SOCK_STREAM || type_ == SOCK_DGRAM);

    assert_eq!(flags, 0); // TODO

    let (num_bytes_read, addr) = match type_ {
        SOCK_DGRAM => {
            let udp_socket = env
                .libc_state
                .socket
                .sockets
                .get(&socket)
                .unwrap()
                .udp_socket
                .as_ref()
                .unwrap();
            let buf = env.mem.bytes_at_mut(buffer.cast(), length);
            let (read, addr) = match udp_socket.recv_from(buf) {
                Ok(n) => n,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No data is ready
                    // TODO: if this happened, take a deep breath and do:
                    // - block guest thread with a new [ThreadBlock] type
                    // - poll for data in thread scheduling part
                    // - write/read/accept/etc data once it is ready
                    // - unblock guest thread
                    set_errno(env, EAGAIN);
                    log_dbg!("recvfrom: UDP socket {} would block, returning EAGAIN", socket);
                    return -1
                }
                Err(e) => panic!("recvfrom: UDP socket {socket} encountered IO error: {e}"),
            };
            if !address.is_null() {
                let guest_addr = sockaddr::from_sockaddr_v4(&addr);
                env.mem.write(address, guest_addr);
                assert_eq!(guest_size_of::<sockaddr>(), env.mem.read(address_len));
                env.mem.write(address_len, guest_size_of::<sockaddr>());
            }
            (read, Ok(addr))
        }
        SOCK_STREAM => {
            assert!(address.is_null());
            assert!(address_len.is_null());
            if env
                .libc_state
                .socket
                .sockets
                .get(&socket)
                .unwrap()
                .tcp_stream
                .is_none()
            {
                set_errno(env, ENOTCONN);
                return -1;
            }
            let mut tcp_stream = env
                .libc_state
                .socket
                .sockets
                .get(&socket)
                .unwrap()
                .tcp_stream
                .as_ref()
                .unwrap();
            let buf = env.mem.bytes_at_mut(buffer.cast(), length);
            let read = match tcp_stream.read(buf) {
                Ok(n) => n,
                Err(ref e) if e.kind() == io::ErrorKind::ConnectionReset => {
                    set_errno(env, ECONNRESET);
                    log!("recvfrom: TCP socket {}: ConnectionReset => -1", socket);
                    return -1;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No data is ready
                    // TODO: if this happened, take a deep breath and do:
                    // - block guest thread with a new [ThreadBlock] type
                    // - poll for data in thread scheduling part
                    // - write/read/accept/etc data once it is ready
                    // - unblock guest thread
                    set_errno(env, EAGAIN);
                    log_dbg!("recvfrom: TCP socket {} would block, returning EAGAIN", socket);
                    return -1
                }
                Err(e) => {
                    let errno = io_error_to_errno(&e);
                    log!("recvfrom: TCP socket {} IO error: {} => errno {}", socket, e, errno);
                    set_errno(env, errno);
                    return -1;
                }
            };
            (read, tcp_stream.peer_addr())
        }
        _ => unreachable!(),
    };
    log_dbg!(
        "recvfrom: Socket {} received {} bytes from addr {:?}",
        socket,
        num_bytes_read,
        addr.ok()
    );
    num_bytes_read.try_into().unwrap()
}

fn send(
    env: &mut Environment,
    socket: i32,
    buffer: MutVoidPtr,
    length: GuestUSize,
    flags: i32,
) -> i32 {
    // TODO: handle errno properly
    set_errno(env, 0);

    let Some(socket_host_object) = State::get(env).sockets.get(&socket) else {
        set_errno(env, EBADF);
        return -1;
    };
    let type_ = socket_host_object.type_;
    assert!(type_ == SOCK_STREAM);
    if socket_host_object.tcp_stream.is_none() {
        set_errno(env, ENOTCONN);
        return -1;
    }

    assert_eq!(flags, 0); // TODO

    let num_bytes_written = match type_ {
        SOCK_STREAM => {
            let mut tcp_stream = env
                .libc_state
                .socket
                .sockets
                .get(&socket)
                .unwrap()
                .tcp_stream
                .as_ref()
                .unwrap();
            let buf = env.mem.bytes_at(buffer.cast(), length);
            match tcp_stream.write(buf) {
                Ok(written) => written,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // TODO: if this happened, take a deep breath and do:
                    // - block guest thread with a new [ThreadBlock] type
                    // - poll for data in thread scheduling part
                    // - write/read/accept/etc data once it is ready
                    // - unblock guest thread
                    set_errno(env, EAGAIN);
                    log_dbg!("send: TCP socket {} would block, returning EAGAIN", socket);
                    return -1
                }
                Err(e) => {
                    let errno = io_error_to_errno(&e);
                    log!("send: TCP socket {} IO error: {} => errno {}", socket, e, errno);
                    set_errno(env, errno);
                    return -1;
                }
            }
        }
        _ => unreachable!(),
    };
    log_dbg!(
        "send: written {} bytes to TCP socket {}",
        num_bytes_written,
        socket
    );
    num_bytes_written.try_into().unwrap()
}

fn sendto(
    env: &mut Environment,
    socket: i32,
    buffer: MutVoidPtr,
    length: GuestUSize,
    flags: i32,
    dest_address: MutPtr<sockaddr>,
    dest_address_len: socklen_t,
) -> i32 {
    // TODO: handle errno properly
    set_errno(env, 0);

    let type_ = State::get(env).sockets.get(&socket).unwrap().type_;
    assert!(type_ == SOCK_DGRAM);

    assert_eq!(flags, 0); // TODO

    assert_eq!(dest_address_len, guest_size_of::<sockaddr>());
    let sockaddr_val = env.mem.read(dest_address);
    let socket_address = sockaddr_val.to_sockaddr_v4();
    log_dbg!(
        "sendto({}, {:?}, {}, {}, {:?} ({:?}, {:?}), {})",
        socket,
        buffer,
        length,
        flags,
        dest_address,
        sockaddr_val,
        socket_address,
        dest_address_len
    );

    let num_bytes_written = match type_ {
        SOCK_DGRAM => {
            if State::get(env)
                .sockets
                .get(&socket)
                .unwrap()
                .udp_socket
                .is_none()
            {
                // For the case of broadcast we allow a lazy host UDP socket
                // creation
                assert!(socket_address.ip().is_broadcast());
                // TODO: is it a correct address to bind?
                let host_socket = UdpSocket::bind("0.0.0.0:0").unwrap();
                assert!(host_socket.local_addr().unwrap().ip().is_unspecified());
                // We set host socket as non-blocking in order to have
                // more control of how and when it's used
                host_socket.set_nonblocking(true).unwrap();
                for &option in &State::get(env).sockets.get(&socket).unwrap().options {
                    if option == SO_BROADCAST {
                        host_socket.set_broadcast(true).unwrap();
                    }
                }
                assert!(host_socket.broadcast().unwrap());
                State::get_mut(env)
                    .sockets
                    .get_mut(&socket)
                    .unwrap()
                    .udp_socket = Some(host_socket);
            }
            let udp_socket = env
                .libc_state
                .socket
                .sockets
                .get(&socket)
                .unwrap()
                .udp_socket
                .as_ref()
                .unwrap();
            if socket_address.ip().is_broadcast() {
                assert!(udp_socket.local_addr().unwrap().ip().is_unspecified());
            }
            let buf = env.mem.bytes_at(buffer.cast(), length);
            match udp_socket.send_to(buf, socket_address) {
                Ok(written) => written,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // TODO: if this happened, take a deep breath and do:
                    // - block guest thread with a new [ThreadBlock] type
                    // - poll for data in thread scheduling part
                    // - write/read/accept/etc data once it is ready
                    // - unblock guest thread
                    set_errno(env, EAGAIN);
                    log_dbg!("sendto: UDP socket {} would block, returning EAGAIN", socket);
                    return -1
                }
                Err(e) => panic!("sendto: Socket {socket} encountered IO error: {e}"),
            }
        }
        _ => unreachable!(),
    };
    log_dbg!(
        "sendto: written {} bytes to UDP socket {} (address {:?})",
        num_bytes_written,
        socket,
        socket_address
    );
    num_bytes_written.try_into().unwrap()
}

const SHUT_RDWR: i32 = 2;
fn shutdown(env: &mut Environment, socket: i32, how: i32) -> i32 {
    log_dbg!("shutdown({}, {})", socket, how);
    set_errno(env, 0);
    let Some(host_object) = State::get(env).sockets.get(&socket) else {
        set_errno(env, EBADF);
        return -1;
    };
    if how == SHUT_RDWR {
        return close(env, socket);
    }
    // Shutting down only one direction: the descriptor stays open.
    if let Some(stream) = host_object.tcp_stream.as_ref() {
        let _ = stream.shutdown(match how {
            0 => std::net::Shutdown::Read,
            _ => std::net::Shutdown::Write,
        });
    }
    0
}

/// Whether a read from the socket would not block (data, a connection to
/// accept, end of stream or an error).
fn socket_read_ready(env: &mut Environment, fd: i32) -> bool {
    let Some(host_object) = State::get(env).sockets.get(&fd) else {
        return false;
    };
    let mut buf = [0u8; 1];
    match host_object.type_ {
        SOCK_DGRAM => match host_object.udp_socket.as_ref() {
            Some(udp_socket) => match udp_socket.peek(&mut buf) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => false,
                _ => true,
            },
            None => false,
        },
        SOCK_STREAM => {
            if let Some(stream) = host_object.tcp_stream.as_ref() {
                !matches!(stream.peek(&mut buf), Err(ref e) if e.kind() == io::ErrorKind::WouldBlock)
            } else if host_object.pending_tcp_stream.is_some() {
                true
            } else if let Some(listener) = host_object.tcp_listener.as_ref() {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(true).unwrap();
                        State::get_mut(env)
                            .sockets
                            .get_mut(&fd)
                            .unwrap()
                            .pending_tcp_stream = Some(stream);
                        true
                    }
                    Err(_) => false,
                }
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Whether a write to the socket would not block.
fn socket_write_ready(env: &mut Environment, fd: i32) -> bool {
    let Some(host_object) = State::get(env).sockets.get(&fd) else {
        return false;
    };
    match host_object.type_ {
        SOCK_STREAM => host_object.tcp_stream.is_some(),
        SOCK_DGRAM => host_object.udp_socket.is_some(),
        _ => false,
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
#[allow(non_camel_case_types)]
struct pollfd {
    fd: i32,
    events: i16,
    revents: i16,
}
unsafe impl SafeRead for pollfd {}

const POLLIN: i16 = 0x1;
const POLLOUT: i16 = 0x4;
const POLLNVAL: i16 = 0x20;
const POLLRDNORM: i16 = 0x40;

fn poll(env: &mut Environment, fds: MutPtr<pollfd>, n_fds: u32, timeout_ms: i32) -> i32 {
    set_errno(env, 0);
    let start = std::time::Instant::now();
    loop {
        let mut ready = 0;
        for i in 0..n_fds {
            let mut entry = env.mem.read(fds + i);
            let (fd, events) = (entry.fd, entry.events);
            let mut revents: i16 = 0;
            if fd >= 0 {
                if State::get(env).sockets.contains_key(&fd) {
                    if events & (POLLIN | POLLRDNORM) != 0 && socket_read_ready(env, fd) {
                        revents |= events & (POLLIN | POLLRDNORM);
                    }
                    if events & POLLOUT != 0 && socket_write_ready(env, fd) {
                        revents |= POLLOUT;
                    }
                } else {
                    revents = crate::libc::posix_io::fd_poll_ready(env, fd, events)
                        .unwrap_or(POLLNVAL);
                }
            }
            entry.revents = revents;
            env.mem.write(fds + i, entry);
            if revents != 0 {
                ready += 1;
            }
        }
        if ready > 0 || timeout_ms == 0 {
            return ready;
        }
        if timeout_ms > 0 && start.elapsed() >= Duration::from_millis(timeout_ms as u64) {
            return 0;
        }
        env.sleep(Duration::from_millis(1));
    }
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(socket(_, _, _)),
    export_c_func!(ioctl(_, _, _)),
    export_c_func!(getsockopt(_, _, _, _, _)),
    export_c_func!(setsockopt(_, _, _, _, _)),
    export_c_func!(getsockname(_, _, _)),
    export_c_func!(getpeername(_, _, _)),
    export_c_func!(poll(_, _, _)),
    export_c_func!(bind(_, _, _)),
    export_c_func!(listen(_, _)),
    export_c_func!(connect(_, _, _)),
    export_c_func!(select(_, _, _, _, _)),
    export_c_func!(accept(_, _, _)),
    export_c_func!(recv(_, _, _, _)),
    export_c_func!(recvfrom(_, _, _, _, _, _)),
    export_c_func!(send(_, _, _, _)),
    export_c_func!(sendto(_, _, _, _, _, _)),
    export_c_func!(shutdown(_, _)),
];

/// A helper to close a socket, not a part of API
pub fn close_socket(env: &mut Environment, socket: i32) -> bool {
    State::get_mut(env).sockets.remove(&socket).is_none()
}
