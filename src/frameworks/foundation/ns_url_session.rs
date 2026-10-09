/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSURLSession`, `NSURLSessionConfiguration` and the data task classes.
//!
//! Tasks are performed by [super::ns_url_connection]'s loading machinery,
//! so completion handlers and delegate callbacks run on the main run loop.
//! Only data tasks (and upload tasks, which are data tasks with a body) exist.

use super::ns_url_connection::{enqueue_task, perform_request, run_completion_handler};
use super::NSInteger;
use crate::objc::blocks::copy_block;
use crate::objc::{
    autorelease, id, msg, msg_class, msg_send_no_type_checking, nil, objc_classes, release, retain,
    ClassExports, HostObject, NSZonePtr,
};
use crate::Environment;

/// `NSURLSessionTaskState`
const NSURLSessionTaskStateRunning: NSInteger = 0;
const NSURLSessionTaskStateSuspended: NSInteger = 1;
const NSURLSessionTaskStateCanceling: NSInteger = 2;
const NSURLSessionTaskStateCompleted: NSInteger = 3;

struct ConfigurationHostObject {
    /// `NSDictionary*`
    additional_headers: id,
    timeout_for_request: f64,
    timeout_for_resource: f64,
}
impl HostObject for ConfigurationHostObject {}

struct SessionHostObject {
    delegate: id,
    /// `NSURLSessionConfiguration*`
    configuration: id,
    invalidated: bool,
}
impl HostObject for SessionHostObject {}

struct TaskHostObject {
    /// `NSURLSession*`
    session: id,
    /// `NSURLRequest*`
    request: id,
    /// Copied completion block, or `nil` if the session delegate is used.
    handler: id,
    /// `NSURLResponse*`, once there is one
    response: id,
    state: NSInteger,
    identifier: u32,
}
impl HostObject for TaskHostObject {}

fn new_task(env: &mut Environment, session: id, request: id, handler: id) -> id {
    let task: id = msg_class![env; NSURLSessionDataTask alloc];
    retain(env, session);
    retain(env, request);
    let handler = if handler == nil {
        nil
    } else {
        copy_block(env, handler)
    };
    let identifier = {
        // Tasks are numbered per run of the app.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    };
    let host_object = env.objc.borrow_mut::<TaskHostObject>(task);
    host_object.session = session;
    host_object.request = request;
    host_object.handler = handler;
    host_object.state = NSURLSessionTaskStateSuspended;
    host_object.identifier = identifier;
    autorelease(env, task)
}

/// Performs a queued task and reports the result to its handler or delegate.
pub(super) fn run_task(env: &mut Environment, task: id) {
    let &TaskHostObject {
        session,
        request,
        handler,
        state,
        ..
    } = env.objc.borrow(task);
    if state != NSURLSessionTaskStateRunning {
        return; // cancelled before it ran
    }
    let delegate = env.objc.borrow::<SessionHostObject>(session).delegate;

    let result = perform_request(env, request);
    // Cancelled from a delegate or while the request was running?
    if env.objc.borrow::<TaskHostObject>(task).state != NSURLSessionTaskStateRunning {
        return;
    }
    match result {
        Ok((response, data)) => {
            retain(env, response);
            let old = std::mem::replace(
                &mut env.objc.borrow_mut::<TaskHostObject>(task).response,
                response,
            );
            release(env, old);
            if handler != nil {
                run_completion_handler(env, handler, data, response, nil);
            } else {
                send_to_session_delegate(env, delegate, "URLSession:dataTask:didReceiveData:", session, task, data);
                send_to_session_delegate(env, delegate, "URLSession:task:didCompleteWithError:", session, task, nil);
            }
        }
        Err(error) => {
            if handler != nil {
                run_completion_handler(env, handler, nil, nil, error);
            } else {
                send_to_session_delegate(env, delegate, "URLSession:task:didCompleteWithError:", session, task, error);
            }
        }
    }
    env.objc.borrow_mut::<TaskHostObject>(task).state = NSURLSessionTaskStateCompleted;
}

fn send_to_session_delegate(
    env: &mut Environment,
    delegate: id,
    name: &str,
    session: id,
    task: id,
    argument: id,
) {
    if delegate == nil {
        return;
    }
    let sel = env
        .objc
        .register_host_selector(name.to_string(), &mut env.mem);
    let responds: bool = msg![env; delegate respondsToSelector:sel];
    if responds {
        let _: () = msg_send_no_type_checking(env, (delegate, sel, session, task, argument));
    }
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSURLSessionConfiguration: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(ConfigurationHostObject {
        additional_headers: nil,
        timeout_for_request: 60.0,
        timeout_for_resource: 7.0 * 24.0 * 60.0 * 60.0,
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

+ (id)defaultSessionConfiguration {
    let new: id = msg![env; this alloc];
    autorelease(env, new)
}
+ (id)ephemeralSessionConfiguration {
    let new: id = msg![env; this alloc];
    autorelease(env, new)
}
+ (id)backgroundSessionConfiguration:(id)_identifier { // NSString *
    let new: id = msg![env; this alloc];
    autorelease(env, new)
}
+ (id)backgroundSessionConfigurationWithIdentifier:(id)_identifier { // NSString *
    let new: id = msg![env; this alloc];
    autorelease(env, new)
}

- (id)copyWithZone:(NSZonePtr)_zone {
    retain(env, this)
}

- (())setHTTPAdditionalHeaders:(id)headers { // NSDictionary *
    retain(env, headers);
    let old = std::mem::replace(
        &mut env.objc.borrow_mut::<ConfigurationHostObject>(this).additional_headers,
        headers,
    );
    release(env, old);
}
- (id)HTTPAdditionalHeaders {
    env.objc.borrow::<ConfigurationHostObject>(this).additional_headers
}
- (())setTimeoutIntervalForRequest:(f64)timeout {
    env.objc.borrow_mut::<ConfigurationHostObject>(this).timeout_for_request = timeout;
}
- (f64)timeoutIntervalForRequest {
    env.objc.borrow::<ConfigurationHostObject>(this).timeout_for_request
}
- (())setTimeoutIntervalForResource:(f64)timeout {
    env.objc.borrow_mut::<ConfigurationHostObject>(this).timeout_for_resource = timeout;
}
- (f64)timeoutIntervalForResource {
    env.objc.borrow::<ConfigurationHostObject>(this).timeout_for_resource
}
- (())setRequestCachePolicy:(NSInteger)_policy {}
- (())setAllowsCellularAccess:(bool)_allows {}
- (())setHTTPMaximumConnectionsPerHost:(NSInteger)_count {}
- (())setHTTPShouldSetCookies:(bool)_should {}
- (())setDiscretionary:(bool)_discretionary {}
- (())setURLCache:(id)_cache {}
- (())setURLCredentialStorage:(id)_storage {}
- (())setHTTPCookieStorage:(id)_storage {}

- (())dealloc {
    let additional_headers = env.objc.borrow::<ConfigurationHostObject>(this).additional_headers;
    release(env, additional_headers);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSURLSession: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(SessionHostObject {
        delegate: nil,
        configuration: nil,
        invalidated: false,
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

+ (id)sharedSession {
    let configuration: id = msg_class![env; NSURLSessionConfiguration defaultSessionConfiguration];
    msg![env; this sessionWithConfiguration:configuration]
}

+ (id)sessionWithConfiguration:(id)configuration { // NSURLSessionConfiguration *
    msg![env; this sessionWithConfiguration:configuration delegate:nil delegateQueue:nil]
}

+ (id)sessionWithConfiguration:(id)configuration // NSURLSessionConfiguration *
                      delegate:(id)delegate
                 delegateQueue:(id)_queue { // NSOperationQueue *
    let new: id = msg![env; this alloc];
    retain(env, configuration);
    retain(env, delegate);
    let host_object = env.objc.borrow_mut::<SessionHostObject>(new);
    host_object.configuration = configuration;
    host_object.delegate = delegate;
    autorelease(env, new)
}

- (id)configuration {
    env.objc.borrow::<SessionHostObject>(this).configuration
}
- (id)delegate {
    env.objc.borrow::<SessionHostObject>(this).delegate
}

- (id)dataTaskWithRequest:(id)request { // NSURLRequest *
    new_task(env, this, request, nil)
}
- (id)dataTaskWithRequest:(id)request // NSURLRequest *
        completionHandler:(id)handler { // void (^)(NSData *, NSURLResponse *, NSError *)
    new_task(env, this, request, handler)
}
- (id)dataTaskWithURL:(id)url { // NSURL *
    let request: id = msg_class![env; NSURLRequest requestWithURL:url];
    new_task(env, this, request, nil)
}
- (id)dataTaskWithURL:(id)url // NSURL *
    completionHandler:(id)handler { // void (^)(NSData *, NSURLResponse *, NSError *)
    let request: id = msg_class![env; NSURLRequest requestWithURL:url];
    new_task(env, this, request, handler)
}

- (id)uploadTaskWithRequest:(id)request // NSURLRequest *
                   fromData:(id)data // NSData *
          completionHandler:(id)handler { // void (^)(NSData *, NSURLResponse *, NSError *)
    // An upload task is a request with a body.
    let mutable_request: id = msg![env; request mutableCopy];
    () = msg![env; mutable_request setHTTPBody:data];
    let task = new_task(env, this, mutable_request, handler);
    release(env, mutable_request);
    task
}

- (())finishTasksAndInvalidate {
    env.objc.borrow_mut::<SessionHostObject>(this).invalidated = true;
}
- (())invalidateAndCancel {
    env.objc.borrow_mut::<SessionHostObject>(this).invalidated = true;
}
- (())flushWithCompletionHandler:(id)_handler {}
- (())resetWithCompletionHandler:(id)_handler {}

- (())dealloc {
    let &SessionHostObject { delegate, configuration, .. } = env.objc.borrow(this);
    release(env, delegate);
    release(env, configuration);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSURLSessionTask: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(TaskHostObject {
        session: nil,
        request: nil,
        handler: nil,
        response: nil,
        state: NSURLSessionTaskStateSuspended,
        identifier: 0,
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (())resume {
    if env.objc.borrow::<TaskHostObject>(this).state != NSURLSessionTaskStateSuspended {
        return;
    }
    env.objc.borrow_mut::<TaskHostObject>(this).state = NSURLSessionTaskStateRunning;
    enqueue_task(env, this);
}
- (())suspend {}
- (())cancel {
    let state = env.objc.borrow::<TaskHostObject>(this).state;
    if state != NSURLSessionTaskStateCompleted {
        env.objc.borrow_mut::<TaskHostObject>(this).state = NSURLSessionTaskStateCanceling;
    }
}

- (NSInteger)state {
    env.objc.borrow::<TaskHostObject>(this).state
}
- (u32)taskIdentifier {
    env.objc.borrow::<TaskHostObject>(this).identifier
}
- (id)response {
    env.objc.borrow::<TaskHostObject>(this).response
}
- (id)originalRequest {
    env.objc.borrow::<TaskHostObject>(this).request
}
- (id)currentRequest {
    env.objc.borrow::<TaskHostObject>(this).request
}
- (())setTaskDescription:(id)_description {}
- (())setPriority:(f32)_priority {}

- (())dealloc {
    let &TaskHostObject { session, request, handler, response, .. } = env.objc.borrow(this);
    release(env, session);
    release(env, request);
    release(env, handler);
    release(env, response);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSURLSessionDataTask: NSURLSessionTask
@end

@implementation NSURLSessionUploadTask: NSURLSessionDataTask
@end

};

