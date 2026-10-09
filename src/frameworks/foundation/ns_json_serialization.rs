/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSJSONSerialization`.
//!
//! JSON text is parsed into `NSDictionary`/`NSArray`/`NSString`/`NSNumber`/
//! `NSNull` objects and written back out from the same classes. Containers are
//! always created as their mutable variants (which are also instances of the
//! immutable classes), whatever the reading options say.

use super::ns_value::NSNumberHostObject;
use super::{ns_data, ns_string, NSUInteger};
use crate::mem::MutPtr;
use crate::objc::{autorelease, id, msg, msg_class, nil, objc_classes, Class, ClassExports};
use crate::Environment;

pub type NSJSONReadingOptions = NSUInteger;
pub const NSJSONReadingAllowFragments: NSJSONReadingOptions = 4;
pub type NSJSONWritingOptions = NSUInteger;
pub const NSJSONWritingPrettyPrinted: NSJSONWritingOptions = 1;

/// `NSCocoaErrorDomain` code for JSON that could not be read or written.
const JSON_ERROR_CODE: i32 = 3840;
const MAX_DEPTH: usize = 512;

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSJSONSerialization: NSObject

+ (id)JSONObjectWithData:(id)data // NSData *
                 options:(NSJSONReadingOptions)options
                   error:(MutPtr<id>)error { // NSError **
    if data == nil {
        set_error(env, error, "No data");
        return nil;
    }
    let bytes = ns_data::to_rust_slice(env, data).to_vec();
    let mut parser = Parser { bytes: &bytes, pos: 0 };
    let result = parser.parse_document(env, options & NSJSONReadingAllowFragments != 0);
    match result {
        Ok(value) => {
            if !error.is_null() {
                env.mem.write(error, nil);
            }
            value
        }
        Err(message) => {
            log!("NSJSONSerialization: could not read JSON: {}", message);
            set_error(env, error, &message);
            nil
        }
    }
}

+ (id)dataWithJSONObject:(id)object
                 options:(NSJSONWritingOptions)options
                   error:(MutPtr<id>)error { // NSError **
    let pretty = options & NSJSONWritingPrettyPrinted != 0;
    let mut out = String::new();
    let result = if is_container(env, object) {
        write_value(env, object, &mut out, pretty, 0)
    } else {
        Err("Top level object is not an NSArray or NSDictionary".to_string())
    };
    match result {
        Ok(()) => {
            if !error.is_null() {
                env.mem.write(error, nil);
            }
            let bytes = out.into_bytes();
            let len: u32 = bytes.len().try_into().unwrap();
            let ptr = env.mem.alloc(len);
            env.mem.bytes_at_mut(ptr.cast(), len).copy_from_slice(&bytes);
            msg_class![env; NSData dataWithBytesNoCopy:ptr length:len]
        }
        Err(message) => {
            log!("NSJSONSerialization: could not write JSON: {}", message);
            set_error(env, error, &message);
            nil
        }
    }
}

+ (bool)isValidJSONObject:(id)object {
    let mut out = String::new();
    is_container(env, object) && write_value(env, object, &mut out, false, 0).is_ok()
}

@end

};

fn set_error(env: &mut Environment, error: MutPtr<id>, message: &str) {
    if error.is_null() {
        return;
    }
    let domain = ns_string::get_static_str(env, "NSCocoaErrorDomain");
    let description = ns_string::from_rust_string(env, message.to_string());
    autorelease(env, description);
    let key = ns_string::get_static_str(env, "NSDebugDescription");
    let info: id = msg_class![env; NSDictionary dictionaryWithObject:description forKey:key];
    let ns_error: id =
        msg_class![env; NSError errorWithDomain:domain code:JSON_ERROR_CODE userInfo:info];
    env.mem.write(error, ns_error);
}

fn known_class(env: &mut Environment, name: &str) -> Class {
    env.objc.get_known_class(name, &mut env.mem)
}

fn is_kind_of(env: &mut Environment, object: id, class_name: &str) -> bool {
    if object == nil {
        return false;
    }
    let class = known_class(env, class_name);
    msg![env; object isKindOfClass:class]
}

fn is_container(env: &mut Environment, object: id) -> bool {
    is_kind_of(env, object, "NSDictionary") || is_kind_of(env, object, "NSArray")
}

// ---------------------------------------------------------------- reading

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn skip_whitespace(&mut self) {
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn error(&self, what: &str) -> String {
        format!("{} at byte {}", what, self.pos)
    }

    fn parse_document(&mut self, env: &mut Environment, allow_fragments: bool) -> Result<id, String> {
        // Skip a UTF-8 byte order mark.
        if self.bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            self.pos = 3;
        }
        self.skip_whitespace();
        if !allow_fragments && !matches!(self.peek(), Some(b'{' | b'[')) {
            return Err(self.error("Top level object is not an array or dictionary"));
        }
        let value = self.parse_value(env, 0)?;
        self.skip_whitespace();
        // Apps often pass the terminating NUL of a C string along with the
        // text, which iOS accepts.
        while self.peek() == Some(0) {
            self.pos += 1;
            self.skip_whitespace();
        }
        if self.pos != self.bytes.len() {
            let rest = &self.bytes[self.pos..self.bytes.len().min(self.pos + 16)];
            return Err(self.error(&format!(
                "Garbage after JSON text ({} of {} bytes used, next bytes {:02x?})",
                self.pos,
                self.bytes.len(),
                rest
            )));
        }
        Ok(value)
    }

    fn parse_value(&mut self, env: &mut Environment, depth: usize) -> Result<id, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("Too many nested arrays or dictionaries"));
        }
        self.skip_whitespace();
        match self.peek() {
            None => Err(self.error("Unexpected end of data")),
            Some(b'{') => self.parse_object(env, depth),
            Some(b'[') => self.parse_array(env, depth),
            Some(b'"') => {
                let string = self.parse_string()?;
                let string = ns_string::from_rust_string(env, string);
                Ok(autorelease(env, string))
            }
            Some(b't') => self.parse_literal(env, "true", |env| {
                msg_class![env; NSNumber numberWithBool:true]
            }),
            Some(b'f') => self.parse_literal(env, "false", |env| {
                msg_class![env; NSNumber numberWithBool:false]
            }),
            Some(b'n') => self.parse_literal(env, "null", |env| msg_class![env; NSNull null]),
            Some(b'-' | b'0'..=b'9') => self.parse_number(env),
            Some(_) => Err(self.error("Unexpected character")),
        }
    }

    fn parse_literal(
        &mut self,
        env: &mut Environment,
        literal: &str,
        make: impl FnOnce(&mut Environment) -> id,
    ) -> Result<id, String> {
        if self.bytes[self.pos..].starts_with(literal.as_bytes()) {
            self.pos += literal.len();
            Ok(make(env))
        } else {
            Err(self.error("Invalid literal"))
        }
    }

    fn parse_number(&mut self, env: &mut Environment) -> Result<id, String> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|b| matches!(b, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
        {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap();
        let is_integer = !text.contains(['.', 'e', 'E']);
        if is_integer {
            if let Ok(int) = text.parse::<i32>() {
                return Ok(msg_class![env; NSNumber numberWithInt:int]);
            }
            if let Ok(long) = text.parse::<i64>() {
                return Ok(msg_class![env; NSNumber numberWithLongLong:long]);
            }
        }
        match text.parse::<f64>() {
            Ok(double) => Ok(msg_class![env; NSNumber numberWithDouble:double]),
            Err(_) => {
                self.pos = start;
                Err(self.error("Invalid number"))
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.error("Truncated unicode escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.error("Invalid unicode escape"))?;
        let value =
            u32::from_str_radix(text, 16).map_err(|_| self.error("Invalid unicode escape"))?;
        self.pos += 4;
        Ok(value)
    }

    /// Parses a string literal, the opening quote being the current byte.
    fn parse_string(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error("Unterminated string"));
            };
            self.pos += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let Some(escape) = self.peek() else {
                        return Err(self.error("Unterminated string"));
                    };
                    self.pos += 1;
                    let ch = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let mut code = self.parse_hex4()?;
                            if (0xD800..0xDC00).contains(&code) {
                                // High surrogate: a low surrogate must follow.
                                if self.bytes.get(self.pos..self.pos + 2) == Some(b"\\u") {
                                    self.pos += 2;
                                    let low = self.parse_hex4()?;
                                    if !(0xDC00..0xE000).contains(&low) {
                                        return Err(self.error("Invalid surrogate pair"));
                                    }
                                    code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                } else {
                                    return Err(self.error("Lone surrogate"));
                                }
                            }
                            char::from_u32(code).ok_or_else(|| self.error("Invalid unicode escape"))?
                        }
                        _ => return Err(self.error("Invalid escape sequence")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                0..=0x1f => return Err(self.error("Unescaped control character in string")),
                _ => out.push(byte),
            }
        }
        String::from_utf8(out).map_err(|_| self.error("Invalid UTF-8 in string"))
    }

    fn parse_array(&mut self, env: &mut Environment, depth: usize) -> Result<id, String> {
        self.pos += 1;
        let array: id = msg_class![env; NSMutableArray array];
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(array);
        }
        loop {
            let value = self.parse_value(env, depth + 1)?;
            () = msg![env; array addObject:value];
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(array);
                }
                _ => return Err(self.error("Expected , or ] in array")),
            }
        }
    }

    fn parse_object(&mut self, env: &mut Environment, depth: usize) -> Result<id, String> {
        self.pos += 1;
        let dict: id = msg_class![env; NSMutableDictionary dictionary];
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(dict);
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("Expected a string key in dictionary"));
            }
            let key = self.parse_string()?;
            let key = ns_string::from_rust_string(env, key);
            autorelease(env, key);
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err(self.error("Expected : after dictionary key"));
            }
            self.pos += 1;
            let value = self.parse_value(env, depth + 1)?;
            () = msg![env; dict setObject:value forKey:key];
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(dict);
                }
                _ => return Err(self.error("Expected , or } in dictionary")),
            }
        }
    }
}

// ---------------------------------------------------------------- writing

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn newline_and_indent(out: &mut String, pretty: bool, depth: usize) {
    if pretty {
        out.push('\n');
        for _ in 0..depth {
            out.push_str("  ");
        }
    }
}

fn write_value(
    env: &mut Environment,
    object: id,
    out: &mut String,
    pretty: bool,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("Too many nested arrays or dictionaries".to_string());
    }
    if object == nil {
        return Err("nil object in JSON".to_string());
    }
    if is_kind_of(env, object, "NSString") {
        let text = ns_string::to_rust_string(env, object).into_owned();
        write_string(out, &text);
    } else if is_kind_of(env, object, "NSNumber") {
        let number = env.objc.borrow::<NSNumberHostObject>(object);
        match number {
            NSNumberHostObject::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            NSNumberHostObject::Float(_) | NSNumberHostObject::Double(_) => {
                let value = number.as_double();
                if !value.is_finite() {
                    return Err("NaN or infinity is not valid JSON".to_string());
                }
                // Floats must still read back as floats.
                let text = format!("{}", value);
                out.push_str(&text);
                if !text.contains(['.', 'e', 'E']) {
                    out.push_str(".0");
                }
            }
            NSNumberHostObject::UnsignedLongLong(value) => out.push_str(&value.to_string()),
            _ => out.push_str(&number.as_long_long().to_string()),
        }
    } else if is_kind_of(env, object, "NSNull") {
        out.push_str("null");
    } else if is_kind_of(env, object, "NSArray") {
        let count: NSUInteger = msg![env; object count];
        if count == 0 {
            out.push_str("[]");
            return Ok(());
        }
        out.push('[');
        for index in 0..count {
            if index > 0 {
                out.push(',');
            }
            newline_and_indent(out, pretty, depth + 1);
            let element: id = msg![env; object objectAtIndex:index];
            write_value(env, element, out, pretty, depth + 1)?;
        }
        newline_and_indent(out, pretty, depth);
        out.push(']');
    } else if is_kind_of(env, object, "NSDictionary") {
        let keys: id = msg![env; object allKeys];
        let count: NSUInteger = msg![env; keys count];
        if count == 0 {
            out.push_str("{}");
            return Ok(());
        }
        out.push('{');
        for index in 0..count {
            if index > 0 {
                out.push(',');
            }
            newline_and_indent(out, pretty, depth + 1);
            let key: id = msg![env; keys objectAtIndex:index];
            if !is_kind_of(env, key, "NSString") {
                return Err("Dictionary key is not a string".to_string());
            }
            let key_text = ns_string::to_rust_string(env, key).into_owned();
            write_string(out, &key_text);
            out.push_str(if pretty { " : " } else { ":" });
            let value: id = msg![env; object objectForKey:key];
            write_value(env, value, out, pretty, depth + 1)?;
        }
        newline_and_indent(out, pretty, depth);
        out.push('}');
    } else {
        return Err("Object type is not valid in JSON".to_string());
    }
    Ok(())
}
