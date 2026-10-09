/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Mach IPC message system.
//!
//! Below messaging interface is the core of Mach's convoluted
//! messaging system for the interprocess communication.
//!
//! So how do we cope with that (considerable) complexity, which involves
//! different processes (or tasks), ports, port's rights, messages, messaging
//! queues, synchronization - all in and out seasoned with dull as hell Apple's
//! own documentation?
//!
//! Well... First, we only have one process (or task) and it will be like this
//! for the time being, so no "real" IPC here (thanks, for god's sake!).
//! Second, the only known use case so far is the Unity's one -
//! mono's mach exception thread which just catches thread
//! exceptions in the loop. (see [mini-darwin.c](https://github.com/mono/mono/blob/62121afbb28f0b62f100ec9a942d10c5e0f4814f/mono/mini/mini-darwin.c#L131))
//!
//! ~~Thus, by a divine benevolence, we stub those functions and
//! hope that no exception will ever happen! amen~~
//!
//! More seriously, as we would prefer to crash on exceptions anyway,
//! it should be fine to just have stubs.
//!
//! Useful resources:
//! - If you want to go deeper, check out "Chapter 4: Inter Process Communication" of [The GNU Mach Reference Manual](https://www.gnu.org/software/hurd/gnumach-doc/mach.pdf).

use crate::dyld::{export_c_func, FunctionExports};
use crate::libc::mach::core_types::{boolean_t, integer_t, natural_t};
use crate::libc::mach::thread_info::{kern_return_t, KERN_SUCCESS};
use crate::mem::MutVoidPtr;
use crate::Environment;

type mach_msg_return_t = kern_return_t;
type mach_port_name_t = natural_t;

type mach_msg_option_t = integer_t;
type mach_msg_size_t = natural_t;
type mach_msg_timeout_t = natural_t;

const MACH_SEND_MSG: mach_msg_option_t = 0x00000001;
const MACH_RCV_MSG: mach_msg_option_t = 0x00000002;
const MACH_RCV_TIMEOUT: mach_msg_option_t = 0x00000100;
const MACH_RCV_TIMED_OUT: mach_msg_return_t = 0x10004003;

#[allow(clippy::too_many_arguments)]
fn mach_msg(
    env: &mut Environment,
    msg: MutVoidPtr, // TODO: use MutPtr<mach_msg_header_t>,
    option: mach_msg_option_t,
    send_size: mach_msg_size_t,
    rcv_size: mach_msg_size_t,
    rcv_name: mach_port_name_t,
    timeout: mach_msg_timeout_t,
    notify: mach_port_name_t,
) -> mach_msg_return_t {
    log_once!("TODO: mach_msg send/rcv");
    log_dbg!(
        "TODO: mach_msg({:?}, {}, {}, {}, {}, {}, {})",
        msg,
        option,
        send_size,
        rcv_size,
        rcv_name,
        timeout,
        notify
    );
    // A receive-only call waits for a message to arrive on `rcv_name`.
    // There is a single task here and nothing in touchHLE ever sends Mach
    // messages to an app-allocated port (exception ports in particular only
    // get messages when the kernel reports a crash), so no message will ever
    // arrive: block like the real kernel would, until the timeout if one was
    // requested, otherwise forever. Returning success here instead hands the
    // caller an unfilled "message" (Crashlytics' exception server thread then
    // processes garbage and crashes).
    if option & MACH_RCV_MSG != 0 && option & MACH_SEND_MSG == 0 {
        if option & MACH_RCV_TIMEOUT != 0 {
            env.sleep(std::time::Duration::from_millis(timeout.into()));
            return MACH_RCV_TIMED_OUT;
        }
        log!(
            "mach_msg: receive on port {:#x} with no possible sender, blocking thread {} forever",
            rcv_name,
            env.current_thread
        );
        loop {
            env.sleep(std::time::Duration::from_secs(3600));
        }
    }
    // Note: Because Unity _do_ check the return value of this function
    // with an assert, we must return a success here.
    // (See [mini-darwin.c](https://github.com/mono/mono/blob/62121afbb28f0b62f100ec9a942d10c5e0f4814f/mono/mini/mini-darwin.c#L139))
    KERN_SUCCESS
}

/// This function is to `Handle kernel-reported thread exception.`
/// See [exc_server](https://web.mit.edu/darwin/src/modules/xnu/osfmk/man/exc_server.html) for more details.
fn exc_server(
    _env: &mut Environment,
    request_msg: MutVoidPtr, // TODO: use MutPtr<mach_msg_header_t>,
    reply_msg: MutVoidPtr,   // TODO: use MutPtr<mach_msg_header_t>,
) -> boolean_t {
    log_dbg!("TODO: exc_server({:?}, {:?})", request_msg, reply_msg);
    // Note: Because Unity _doesn't_ check the return value of this function
    // with an assert, we can just return a false here.
    // (See [mini-darwin.c](https://github.com/mono/mono/blob/62121afbb28f0b62f100ec9a942d10c5e0f4814f/mono/mini/mini-darwin.c#L142))
    1 // FALSE
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(mach_msg(_, _, _, _, _, _, _)),
    export_c_func!(exc_server(_, _)),
];
