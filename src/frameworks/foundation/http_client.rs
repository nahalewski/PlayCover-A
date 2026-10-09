/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! A small blocking HTTP/1.1 client for the host side of `NSURLConnection`
//! and `NSURLSession`.
//!
//! It supports plain `http://` URLs: request bodies, redirects, chunked and
//! `Content-Length` responses. TODO: TLS (`https://` URLs fail with
//! [HttpError::SecureConnectionFailed]), cookies and content encodings.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

pub struct HttpResponse {
    /// The URL that was finally fetched, after redirects.
    pub url: String,
    pub status: i32,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Failures, matching `NSURLError` codes.
#[derive(Debug)]
pub enum HttpError {
    BadUrl,
    UnsupportedUrl,
    TimedOut,
    CannotFindHost,
    CannotConnectToHost,
    NetworkConnectionLost,
    SecureConnectionFailed,
    BadServerResponse,
    TooManyRedirects,
}
impl HttpError {
    pub fn ns_url_error_code(&self) -> i32 {
        match self {
            HttpError::BadUrl => -1000,
            HttpError::TimedOut => -1001,
            HttpError::UnsupportedUrl => -1002,
            HttpError::CannotFindHost => -1003,
            HttpError::CannotConnectToHost => -1004,
            HttpError::NetworkConnectionLost => -1005,
            HttpError::BadServerResponse => -1011,
            HttpError::TooManyRedirects => -1007,
            HttpError::SecureConnectionFailed => -1200,
        }
    }
    pub fn description(&self) -> &'static str {
        match self {
            HttpError::BadUrl => "bad URL",
            HttpError::TimedOut => "The request timed out.",
            HttpError::UnsupportedUrl => "unsupported URL",
            HttpError::CannotFindHost => "A server with the specified hostname could not be found.",
            HttpError::CannotConnectToHost => "Could not connect to the server.",
            HttpError::NetworkConnectionLost => "The network connection was lost.",
            HttpError::BadServerResponse => "The server returned an invalid response.",
            HttpError::TooManyRedirects => "too many HTTP redirects",
            HttpError::SecureConnectionFailed => "A secure connection could not be made.",
        }
    }
}

struct ParsedUrl {
    host: String,
    port: u16,
    /// Path and query, always starting with `/`.
    path: String,
}

fn parse_url(url: &str) -> Result<ParsedUrl, HttpError> {
    let (scheme, rest) = url.split_once("://").ok_or(HttpError::BadUrl)?;
    if scheme.eq_ignore_ascii_case("https") {
        return Err(HttpError::SecureConnectionFailed);
    }
    if !scheme.eq_ignore_ascii_case("http") {
        return Err(HttpError::UnsupportedUrl);
    }
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(index) => {
            let (authority, path) = rest.split_at(index);
            let path = path.split('#').next().unwrap_or("");
            let path = if path.starts_with('/') {
                path.to_string()
            } else {
                format!("/{path}")
            };
            (authority, path)
        }
        None => (rest, "/".to_string()),
    };
    // Credentials in the URL are not sent.
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            (host, port.parse::<u16>().map_err(|_| HttpError::BadUrl)?)
        }
        _ => (authority, 80),
    };
    if host.is_empty() {
        return Err(HttpError::BadUrl);
    }
    Ok(ParsedUrl {
        host: host.to_string(),
        port,
        path,
    })
}

/// Resolves a `Location` header against the URL it came with.
fn resolve_location(base: &str, location: &str) -> String {
    if location.contains("://") {
        return location.to_string();
    }
    let Some((scheme, rest)) = base.split_once("://") else {
        return location.to_string();
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if location.starts_with('/') {
        return format!("{scheme}://{authority}{location}");
    }
    let path = &rest[authority_end..];
    let directory = match path.rfind('/') {
        Some(index) => &path[..=index],
        None => "/",
    };
    format!("{scheme}://{authority}{directory}{location}")
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn read_error(error: &std::io::Error) -> HttpError {
    match error.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => HttpError::TimedOut,
        _ => HttpError::NetworkConnectionLost,
    }
}

/// Performs a request and follows redirects. `timeout` applies to connecting
/// and to every wait for data.
pub fn request(
    method: &str,
    url: &str,
    request_headers: &[(String, String)],
    body: &[u8],
    timeout: Duration,
) -> Result<HttpResponse, HttpError> {
    let mut method = method.to_string();
    let mut body = body.to_vec();
    let mut url = url.to_string();
    for _ in 0..8 {
        let response = single_request(&method, &url, request_headers, &body, timeout)?;
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            if let Some(location) = header(&response.headers, "Location") {
                url = resolve_location(&response.url, location);
                if response.status != 307 && response.status != 308 && method != "HEAD" {
                    // The new request is a GET without a body.
                    method = "GET".to_string();
                    body.clear();
                }
                continue;
            }
        }
        return Ok(response);
    }
    Err(HttpError::TooManyRedirects)
}

fn single_request(
    method: &str,
    url: &str,
    request_headers: &[(String, String)],
    body: &[u8],
    timeout: Duration,
) -> Result<HttpResponse, HttpError> {
    if let Some((_, rest)) = url.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = authority.rsplit('@').next().unwrap_or(authority);
        if crate::ad_blocklist::is_blocked_host(host) {
            log!("Blocked advertising request to {:?}", host);
            return Err(HttpError::CannotFindHost);
        }
    }
    let parsed = parse_url(url)?;
    let deadline = Instant::now() + timeout.max(Duration::from_secs(1));

    let address = (parsed.host.as_str(), parsed.port)
        .to_socket_addrs()
        .map_err(|_| HttpError::CannotFindHost)?
        .find(|address| address.is_ipv4())
        .ok_or(HttpError::CannotFindHost)?;
    let mut stream = TcpStream::connect_timeout(&address, timeout).map_err(|error| {
        if error.kind() == std::io::ErrorKind::TimedOut {
            HttpError::TimedOut
        } else {
            HttpError::CannotConnectToHost
        }
    })?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    // Request.
    let mut head = format!("{} {} HTTP/1.1\r\n", method, parsed.path);
    if parsed.port == 80 {
        head.push_str(&format!("Host: {}\r\n", parsed.host));
    } else {
        head.push_str(&format!("Host: {}:{}\r\n", parsed.host, parsed.port));
    }
    head.push_str("Connection: close\r\nAccept-Encoding: identity\r\n");
    let mut has_accept = false;
    let mut has_user_agent = false;
    for (name, value) in request_headers {
        if ["host", "connection", "accept-encoding", "content-length"]
            .iter()
            .any(|skipped| name.eq_ignore_ascii_case(skipped))
        {
            continue;
        }
        has_accept |= name.eq_ignore_ascii_case("accept");
        has_user_agent |= name.eq_ignore_ascii_case("user-agent");
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if !has_accept {
        head.push_str("Accept: */*\r\n");
    }
    if !has_user_agent {
        head.push_str("User-Agent: touchHLE CFNetwork/548.0 Darwin/11.0.0\r\n");
    }
    if !body.is_empty() || matches!(method, "POST" | "PUT" | "PATCH") {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body))
        .map_err(|error| read_error(&error))?;

    // Response head.
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break index;
        }
        if Instant::now() >= deadline + timeout {
            return Err(HttpError::TimedOut);
        }
        let count = stream.read(&mut chunk).map_err(|error| read_error(&error))?;
        if count == 0 {
            return Err(HttpError::NetworkConnectionLost);
        }
        buffer.extend_from_slice(&chunk[..count]);
    };
    let head_text = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head_text.split("\r\n");
    let status_line = lines.next().ok_or(HttpError::BadServerResponse)?;
    let status: i32 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or(HttpError::BadServerResponse)?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    let mut rest: Vec<u8> = buffer[head_end + 4..].to_vec();

    // Response body.
    let no_body = method == "HEAD" || status == 204 || status == 304 || (100..200).contains(&status);
    let chunked = header(&headers, "Transfer-Encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"));
    let content_length: Option<usize> =
        header(&headers, "Content-Length").and_then(|value| value.parse().ok());
    let body = if no_body {
        Vec::new()
    } else if chunked {
        let mut decoded = Vec::new();
        loop {
            // Chunk size line.
            let line_end = loop {
                if let Some(index) = rest.windows(2).position(|window| window == b"\r\n") {
                    break index;
                }
                let count = stream.read(&mut chunk).map_err(|error| read_error(&error))?;
                if count == 0 {
                    return Err(HttpError::NetworkConnectionLost);
                }
                rest.extend_from_slice(&chunk[..count]);
            };
            let size_text = String::from_utf8_lossy(&rest[..line_end]).into_owned();
            let size_text = size_text.split(';').next().unwrap_or("").trim().to_string();
            let size = usize::from_str_radix(&size_text, 16)
                .map_err(|_| HttpError::BadServerResponse)?;
            rest.drain(..line_end + 2);
            if size == 0 {
                break;
            }
            while rest.len() < size + 2 {
                let count = stream.read(&mut chunk).map_err(|error| read_error(&error))?;
                if count == 0 {
                    return Err(HttpError::NetworkConnectionLost);
                }
                rest.extend_from_slice(&chunk[..count]);
            }
            decoded.extend_from_slice(&rest[..size]);
            rest.drain(..size + 2);
        }
        decoded
    } else if let Some(length) = content_length {
        while rest.len() < length {
            let count = stream.read(&mut chunk).map_err(|error| read_error(&error))?;
            if count == 0 {
                return Err(HttpError::NetworkConnectionLost);
            }
            rest.extend_from_slice(&chunk[..count]);
        }
        rest.truncate(length);
        rest
    } else {
        // Until the server closes the connection.
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => rest.extend_from_slice(&chunk[..count]),
                Err(error) => {
                    if rest.is_empty() {
                        return Err(read_error(&error));
                    }
                    break;
                }
            }
        }
        rest
    };

    Ok(HttpResponse {
        url: url.to_string(),
        status,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        let url = parse_url("http://example.com:8080/a/b?c=d#frag").unwrap();
        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, 8080);
        assert_eq!(url.path, "/a/b?c=d");
        let url = parse_url("http://example.com").unwrap();
        assert_eq!((url.port, url.path.as_str()), (80, "/"));
        assert!(matches!(
            parse_url("https://example.com/"),
            Err(HttpError::SecureConnectionFailed)
        ));
        assert!(matches!(parse_url("ftp://example.com/"), Err(HttpError::UnsupportedUrl)));
    }

    #[test]
    fn locations() {
        assert_eq!(
            resolve_location("http://a.com/x/y?q=1", "/z"),
            "http://a.com/z"
        );
        assert_eq!(
            resolve_location("http://a.com/x/y", "w"),
            "http://a.com/x/w"
        );
        assert_eq!(
            resolve_location("http://a.com/x", "http://b.com/"),
            "http://b.com/"
        );
    }
}
