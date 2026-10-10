/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSURLConnection`, and the loading machinery that `NSURLSession` shares.
//!
//! Requests are run by [super::http_client] on the host, on the main run
//! loop's thread: asynchronous loads are queued and performed (and their
//! delegate callbacks or completion handlers invoked) when the app next
//! returns to the run loop. TODO: `https://` (see [super::http_client]).

use super::http_client::{self, HttpError};
use super::{ns_data, ns_string, ns_url_response, NSInteger};
use crate::abi::{CallFromHost, GuestFunction};
use crate::dyld::{ConstantExports, HostConstant};
use crate::environment::Environment;
use crate::mem::{MutPtr, Ptr};
use crate::objc::blocks::copy_block;
use crate::objc::{
    autorelease, id, msg, msg_class, msg_send_no_type_checking, nil, objc_classes, release, retain,
    ClassExports, HostObject, NSZonePtr,
};
use std::collections::VecDeque;
use std::time::Duration;

const NSURLErrorDomain: &str = "NSURLErrorDomain";
const NSURLErrorFailingURLStringErrorKey: &str = "NSErrorFailingURLStringKey";
const NSLocalizedDescriptionKey: &str = "NSLocalizedDescriptionKey";

/// `NSURLErrorNotConnectedToInternet`
const NOT_CONNECTED_TO_INTERNET: i32 = -1009;

pub const CONSTANTS: ConstantExports = &[
    (
        "_NSURLErrorDomain",
        HostConstant::NSString(NSURLErrorDomain),
    ),
    (
        "_NSURLErrorFailingURLStringErrorKey",
        HostConstant::NSString(NSURLErrorFailingURLStringErrorKey),
    ),
];

/// A load that has been started but whose result has not been delivered.
#[derive(Debug)]
enum Pending {
    /// An `NSURLConnection` with a delegate.
    Connection(id),
    /// `+sendAsynchronousRequest:queue:completionHandler:`: the request and a
    /// copied completion block.
    Block { request: id, handler: id },
    /// An `NSURLSessionTask`.
    Task(id),
}

#[derive(Default)]
pub struct State {
    pending: VecDeque<Pending>,
}

fn enqueue(env: &mut Environment, pending: Pending) {
    env.framework_state
        .foundation
        .ns_url_connection
        .pending
        .push_back(pending);
}

/// Queues an `NSURLSessionTask` to run on the run loop.
pub(super) fn enqueue_task(env: &mut Environment, task: id) {
    retain(env, task);
    enqueue(env, Pending::Task(task));
}

/// For use by `NSRunLoop` (main run loop only): performs the next queued load
/// and delivers its result.
pub fn handle_pending_loads(env: &mut Environment) {
    let Some(pending) = env
        .framework_state
        .foundation
        .ns_url_connection
        .pending
        .pop_front()
    else {
        return;
    };
    match pending {
        Pending::Connection(connection) => {
            finish_connection(env, connection);
            release(env, connection);
        }
        Pending::Block { request, handler } => {
            let result = perform_request(env, request);
            let (response, data, error) = match result {
                Ok((response, data)) => (response, data, nil),
                Err(error) => (nil, nil, error),
            };
            // NSURLConnection's handler takes (response, data, error).
            run_completion_handler(env, handler, response, data, error);
            release(env, handler);
            release(env, request);
        }
        Pending::Task(task) => {
            super::ns_url_session::run_task(env, task);
            release(env, task);
        }
    }
}

/// Calls a completion handler block with three object arguments: for
/// `NSURLSession` they are (data, response, error), for `NSURLConnection`
/// (response, data, error).
pub(super) fn run_completion_handler(
    env: &mut Environment,
    handler: id,
    first: id,
    second: id,
    third: id,
) {
    if handler == nil {
        return;
    }
    let invoke: u32 = env
        .mem
        .read(Ptr::<u32, false>::from_bits(handler.to_bits() + 12));
    let invoke = GuestFunction::from_addr_with_thumb_bit(invoke);
    let () = invoke.call_from_host(env, (handler, first, second, third));
}

/// An autoreleased `NSError` in `NSURLErrorDomain`.
fn make_url_error(env: &mut Environment, code: i32, description: &str, url: &str) -> id {
    let domain = ns_string::get_static_str(env, NSURLErrorDomain);
    let description = ns_string::from_rust_string(env, description.to_string());
    let url = ns_string::from_rust_string(env, url.to_string());
    let description_key = ns_string::get_static_str(env, NSLocalizedDescriptionKey);
    let url_key = ns_string::get_static_str(env, NSURLErrorFailingURLStringErrorKey);
    let user_info = super::ns_dictionary::dict_from_keys_and_objects(
        env,
        &[(description_key, description), (url_key, url)],
    );
    release(env, description);
    release(env, url);
    let error: id = msg_class![env; NSError errorWithDomain:domain code:(code as NSInteger) userInfo:user_info];
    release(env, user_info);
    error
}

/// Runs the request now. On success, returns the (autoreleased)
/// `NSHTTPURLResponse` and `NSData`; on failure the `NSError`.
pub(super) fn perform_request(env: &mut Environment, request: id) -> Result<(id, id), id> {
    if request == nil {
        return Err(make_url_error(env, -1000, "bad URL", ""));
    }
    let url: id = msg![env; request URL];
    if url == nil {
        return Err(make_url_error(env, -1000, "bad URL", ""));
    }
    let url_string: id = msg![env; url absoluteString];
    let url_string = ns_string::to_rust_string(env, url_string).into_owned();

    if !env.options.network_access {
        log!(
            "Network access is disabled, request for '{}' fails (offline)",
            url_string
        );
        return Err(make_url_error(
            env,
            NOT_CONNECTED_TO_INTERNET,
            "The Internet connection appears to be offline.",
            &url_string,
        ));
    }

    let method: id = msg![env; request HTTPMethod];
    let method = if method == nil {
        "GET".to_string()
    } else {
        ns_string::to_rust_string(env, method).into_owned()
    };
    let mut headers: Vec<(String, String)> = Vec::new();
    let header_dict: id = msg![env; request allHTTPHeaderFields];
    if header_dict != nil {
        let keys: id = msg![env; header_dict allKeys];
        let count: u32 = msg![env; keys count];
        for index in 0..count {
            let key: id = msg![env; keys objectAtIndex:index];
            let value: id = msg![env; header_dict objectForKey:key];
            if value != nil {
                headers.push((
                    ns_string::to_rust_string(env, key).into_owned(),
                    ns_string::to_rust_string(env, value).into_owned(),
                ));
            }
        }
    }
    let body: id = msg![env; request HTTPBody];
    let body = if body == nil {
        Vec::new()
    } else {
        ns_data::to_rust_slice(env, body).to_vec()
    };
    let timeout: f64 = msg![env; request timeoutInterval];
    let timeout = Duration::from_secs_f64(timeout.clamp(1.0, 120.0));

    log!("{} {} ({} body bytes)", method, url_string, body.len());
    match http_client::request(&method, &url_string, &headers, &body, timeout) {
        Ok(response) => {
            log!(
                "{} {} => HTTP {} ({} bytes): {}",
                method,
                url_string,
                response.status,
                response.body.len(),
                String::from_utf8_lossy(&response.body[..response.body.len().min(160)])
                    .escape_debug()
            );
            let ns_response = ns_url_response::make_http_response(
                env,
                &response.url,
                response.status,
                &response.headers,
                response.body.len(),
            );
            let data: id = if response.body.is_empty() {
                if url_string.contains("gamevil.com") {
                    let fallback = b"0";
                    let length: u32 = fallback.len() as u32;
                    let ptr = env.mem.alloc(length);
                    env.mem
                        .bytes_at_mut(ptr.cast(), length)
                        .copy_from_slice(fallback);
                    msg_class![env; NSData dataWithBytesNoCopy:ptr length:length]
                } else {
                    msg_class![env; NSData data]
                }
            } else {
                let length: u32 = response.body.len().try_into().unwrap();
                let ptr = env.mem.alloc(length);
                env.mem
                    .bytes_at_mut(ptr.cast(), length)
                    .copy_from_slice(&response.body);
                msg_class![env; NSData dataWithBytesNoCopy:ptr length:length]
            };
            Ok((ns_response, data))
        }
        Err(error) => {
            log!("{} {} failed: {:?}", method, url_string, error);
            Err(make_url_error(
                env,
                error.ns_url_error_code(),
                error.description(),
                &url_string,
            ))
        }
    }
}

struct ConnectionHostObject {
    /// `NSURLRequest*`
    request: id,
    delegate: id,
    started: bool,
    finished: bool,
}
impl HostObject for ConnectionHostObject {}

/// Sends `-name:` or `-name:argument:` to the delegate if it implements it.
fn send_to_delegate(env: &mut Environment, delegate: id, name: &str, connection: id, argument: Option<id>) {
    if delegate == nil {
        return;
    }
    let sel = env
        .objc
        .register_host_selector(name.to_string(), &mut env.mem);
    let responds: bool = msg![env; delegate respondsToSelector:sel];
    if !responds {
        return;
    }
    match argument {
        Some(argument) => {
            let _: () = msg_send_no_type_checking(env, (delegate, sel, connection, argument));
        }
        None => {
            let _: () = msg_send_no_type_checking(env, (delegate, sel, connection));
        }
    }
}

fn finish_connection(env: &mut Environment, connection: id) {
    let &ConnectionHostObject {
        request,
        delegate,
        finished,
        ..
    } = env.objc.borrow(connection);
    if finished {
        return;
    }
    match perform_request(env, request) {
        Ok((response, data)) => {
            send_to_delegate(env, delegate, "connection:didReceiveResponse:", connection, Some(response));
            let length: u32 = msg![env; data length];
            if length > 0 {
                send_to_delegate(env, delegate, "connection:didReceiveData:", connection, Some(data));
            }
            if env.objc.borrow::<ConnectionHostObject>(connection).finished {
                return; // cancelled by the delegate
            }
            send_to_delegate(env, delegate, "connectionDidFinishLoading:", connection, None);
        }
        Err(error) => {
            send_to_delegate(env, delegate, "connection:didFailWithError:", connection, Some(error));
        }
    }
    env.objc.borrow_mut::<ConnectionHostObject>(connection).finished = true;
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSURLConnection: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(ConnectionHostObject {
        request: nil,
        delegate: nil,
        started: false,
        finished: false,
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

+ (id)sendSynchronousRequest:(id)request // NSURLRequest *
           returningResponse:(MutPtr<id>)response // NSURLResponse **
                       error:(MutPtr<id>)out_error { // NSError **
    match perform_request(env, request) {
        Ok((ns_response, data)) => {
            if !response.is_null() {
                env.mem.write(response, ns_response);
            }
            if !out_error.is_null() {
                env.mem.write(out_error, nil);
            }
            data
        }
        Err(error) => {
            if !response.is_null() {
                env.mem.write(response, nil);
            }
            if !out_error.is_null() {
                env.mem.write(out_error, error);
            }
            nil
        }
    }
}

+ (())sendAsynchronousRequest:(id)request // NSURLRequest *
                        queue:(id)_queue // NSOperationQueue *
            completionHandler:(id)handler { // void (^)(NSURLResponse*, NSData*, NSError*)
    // The completion handler of this API has the response first, then the data.
    let _ = (request, handler);
    let handler = copy_block(env, handler);
    retain(env, request);
    enqueue(env, Pending::Block { request, handler });
}

+ (id)connectionWithRequest:(id)request // NSURLRequest *
                   delegate:(id)delegate {
    let new: id = msg![env; this alloc];
    let new: id = msg![env; new initWithRequest:request delegate:delegate];
    autorelease(env, new)
}

- (id)initWithRequest:(id)request // NSURLRequest *
             delegate:(id)delegate {
    msg![env; this initWithRequest:request delegate:delegate startImmediately:true]
}

- (id)initWithRequest:(id)request // NSURLRequest *
             delegate:(id)delegate
     startImmediately:(bool)start_immediately {
    retain(env, request);
    retain(env, delegate);
    let host_object = env.objc.borrow_mut::<ConnectionHostObject>(this);
    host_object.request = request;
    host_object.delegate = delegate;
    if start_immediately {
        () = msg![env; this start];
    }
    this
}

- (())start {
    if env.objc.borrow::<ConnectionHostObject>(this).started {
        return;
    }
    env.objc.borrow_mut::<ConnectionHostObject>(this).started = true;
    retain(env, this); // owned by the queue until the result is delivered
    enqueue(env, Pending::Connection(this));
}

- (())cancel {
    env.objc.borrow_mut::<ConnectionHostObject>(this).finished = true;
}

- (())scheduleInRunLoop:(id)_run_loop forMode:(id)_mode {}
- (())unscheduleFromRunLoop:(id)_run_loop forMode:(id)_mode {}
- (())setDelegateQueue:(id)_queue {}

- (id)originalRequest {
    env.objc.borrow::<ConnectionHostObject>(this).request
}
- (id)currentRequest {
    env.objc.borrow::<ConnectionHostObject>(this).request
}

- (())dealloc {
    let &ConnectionHostObject { request, delegate, .. } = env.objc.borrow(this);
    release(env, request);
    release(env, delegate);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

};
