/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSURLResponse` and `NSHTTPURLResponse`.

use super::{ns_string, NSInteger};
use crate::objc::{
    autorelease, id, msg, msg_class, nil, objc_classes, release, retain, ClassExports, HostObject,
    NSZonePtr,
};
use crate::Environment;

struct ResponseHostObject {
    /// `NSURL*`
    url: id,
    status: NSInteger,
    /// `NSDictionary*` of `NSString*` to `NSString*`
    headers: id,
    /// `NSString*`
    mime_type: id,
    expected_content_length: i64,
    /// `NSString*`
    text_encoding_name: id,
}
impl HostObject for ResponseHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSURLResponse: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let headers: id = msg_class![env; NSDictionary new];
    let host_object = Box::new(ResponseHostObject {
        url: nil,
        status: 200,
        headers,
        mime_type: nil,
        expected_content_length: -1,
        text_encoding_name: nil,
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithURL:(id)url // NSURL *
          MIMEType:(id)mime_type // NSString *
expectedContentLength:(NSInteger)length
  textEncodingName:(id)encoding { // NSString *
    retain(env, url);
    retain(env, mime_type);
    retain(env, encoding);
    let host_object = env.objc.borrow_mut::<ResponseHostObject>(this);
    host_object.url = url;
    host_object.mime_type = mime_type;
    host_object.expected_content_length = length as i64;
    host_object.text_encoding_name = encoding;
    this
}

- (id)URL {
    env.objc.borrow::<ResponseHostObject>(this).url
}
- (id)MIMEType {
    env.objc.borrow::<ResponseHostObject>(this).mime_type
}
- (i64)expectedContentLength {
    env.objc.borrow::<ResponseHostObject>(this).expected_content_length
}
- (id)textEncodingName {
    env.objc.borrow::<ResponseHostObject>(this).text_encoding_name
}
- (id)suggestedFilename {
    let url = env.objc.borrow::<ResponseHostObject>(this).url;
    let name: id = msg![env; url lastPathComponent];
    name
}

- (())dealloc {
    let &ResponseHostObject {
        url,
        headers,
        mime_type,
        text_encoding_name,
        ..
    } = env.objc.borrow(this);
    release(env, url);
    release(env, headers);
    release(env, mime_type);
    release(env, text_encoding_name);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSHTTPURLResponse: NSURLResponse

- (id)initWithURL:(id)url // NSURL *
       statusCode:(NSInteger)status
      HTTPVersion:(id)_version // NSString *
     headerFields:(id)headers { // NSDictionary *
    retain(env, url);
    let headers_copy: id = if headers == nil {
        msg_class![env; NSDictionary new]
    } else {
        msg![env; headers copy]
    };
    let host_object = env.objc.borrow_mut::<ResponseHostObject>(this);
    let old_headers = std::mem::replace(&mut host_object.headers, headers_copy);
    host_object.url = url;
    host_object.status = status;
    release(env, old_headers);
    this
}

- (NSInteger)statusCode {
    env.objc.borrow::<ResponseHostObject>(this).status
}

- (id)allHeaderFields {
    env.objc.borrow::<ResponseHostObject>(this).headers
}

- (id)valueForHTTPHeaderField:(id)field { // NSString *
    let wanted = ns_string::to_rust_string(env, field).to_ascii_lowercase();
    let headers = env.objc.borrow::<ResponseHostObject>(this).headers;
    let keys: id = msg![env; headers allKeys];
    let count: u32 = msg![env; keys count];
    for index in 0..count {
        let key: id = msg![env; keys objectAtIndex:index];
        if ns_string::to_rust_string(env, key).to_ascii_lowercase() == wanted {
            return msg![env; headers objectForKey:key];
        }
    }
    nil
}

@end

};

/// Builds an `NSHTTPURLResponse` (autoreleased) from a finished host request.
pub fn make_http_response(
    env: &mut Environment,
    url: &str,
    status: i32,
    headers: &[(String, String)],
    body_length: usize,
) -> id {
    let url_string = ns_string::from_rust_string(env, url.to_string());
    let url: id = msg_class![env; NSURL URLWithString:url_string];
    release(env, url_string);

    let mut keys_and_objects: Vec<(id, id)> = Vec::new();
    for (name, value) in headers {
        let key = ns_string::from_rust_string(env, name.clone());
        let value = ns_string::from_rust_string(env, value.clone());
        keys_and_objects.push((key, value));
    }
    let dictionary = super::ns_dictionary::dict_from_keys_and_objects(env, &keys_and_objects);
    for (key, value) in keys_and_objects {
        release(env, key);
        release(env, value);
    }

    let response: id = msg_class![env; NSHTTPURLResponse alloc];
    let version = ns_string::get_static_str(env, "HTTP/1.1");
    let response: id =
        msg![env; response initWithURL:url statusCode:status HTTPVersion:version headerFields:dictionary];
    release(env, dictionary);

    // Content-Type: "text/html; charset=UTF-8"
    let content_type = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Content-Type"))
        .map(|(_, value)| value.clone());
    let mime = content_type
        .as_ref()
        .map(|value| value.split(';').next().unwrap_or("").trim().to_string());
    let encoding = content_type.as_ref().and_then(|value| {
        value
            .to_ascii_lowercase()
            .split("charset=")
            .nth(1)
            .map(|charset| charset.split(';').next().unwrap_or("").trim().to_string())
    });
    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Content-Length"))
        .and_then(|(_, value)| value.parse::<i64>().ok())
        .unwrap_or(body_length as i64);
    let mime_id = match mime {
        Some(mime) => ns_string::from_rust_string(env, mime),
        None => nil,
    };
    let encoding_id = match encoding {
        Some(encoding) => ns_string::from_rust_string(env, encoding),
        None => nil,
    };
    let host_object = env.objc.borrow_mut::<ResponseHostObject>(response);
    host_object.mime_type = mime_id;
    host_object.text_encoding_name = encoding_id;
    host_object.expected_content_length = length;
    autorelease(env, response)
}
