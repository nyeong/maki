//! HTTP module
//!
//! request, response, parse, serialize, ...
//! This module does not know Maki routes or web error policy.

use std::{collections::HashMap, fmt::Display};

#[derive(Debug, PartialEq)]
pub(crate) enum Error {
    InvalidRequest,
    InvalidVersion,
    InvalidMethod,
    InvalidTarget,
    InvalidHeader,
    EmptyRequest,
    // TODO: Unsupported..., Malformed...
}

use Error::*;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum StatusCode {
    Ok, // 200
    #[allow(dead_code)]
    MovedPermanently, // 301
    Found, // 302
    NotFound, // 404
    BadRequest, // 400
    InternalServerError, // 500
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum Version {
    Http1_1,
}

impl std::str::FromStr for Version {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_version(s)
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Version::Http1_1 => write!(f, "HTTP/1.1"),
        }
    }
}

impl Headers {
    pub(crate) fn new() -> Self {
        Self(HashMap::new())
    }

    pub(crate) fn insert(&mut self, key: String, value: String) {
        self.0.insert(key.to_lowercase(), value);
    }

    #[allow(dead_code)]
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.0.get(&key.to_lowercase()).map(|s| s.as_str())
    }
}

/// Parses a raw HTTP request string into a [`HttpRequest`] struct.
pub(crate) fn parse_request(request: &str) -> Result<Request, Error> {
    let mut lines = request.lines();
    let first_line = lines.next().ok_or(EmptyRequest)?;

    let (method, target, version) = parse_request_line(first_line)?;

    let headers = parse_request_headers(&mut lines)?;

    Ok(Request {
        method,
        target,
        version,
        headers,
        body: vec![],
    })
}
#[derive(Debug, PartialEq)]
pub(crate) struct Request {
    method: Method,
    target: String,
    version: Version,
    headers: Headers,
    body: Vec<u8>,
}

impl Request {
    #[allow(dead_code)]
    pub(crate) fn new(method: Method, target: impl Into<String>) -> Self {
        Self {
            method,
            target: target.into(),
            version: Version::Http1_1,
            headers: Headers::new(),
            body: vec![],
        }
    }

    #[allow(dead_code)]
    pub(crate) fn get(target: impl Into<String>) -> Self {
        Self::new(Method::Get, target.into())
    }

    pub(crate) fn target(&self) -> &str {
        &self.target
    }
}

/// Parses an HTTP request-line.
pub(crate) fn parse_request_line(line: &str) -> Result<(Method, String, Version), Error> {
    let mut parts = line.split_whitespace();
    let method = parts.next().ok_or(InvalidMethod)?.parse()?;
    let target = parts.next().ok_or(InvalidTarget)?.to_string();
    let version = parts.next().ok_or(InvalidVersion)?.parse()?;
    if parts.next().is_some() {
        return Err(InvalidRequest);
    }
    Ok((method, target, version))
}

pub(crate) fn parse_request_headers<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
) -> Result<Headers, Error> {
    let mut headers = Headers::new();

    for line in lines {
        if line.is_empty() {
            break;
        }
        let (key, value) = line.split_once(':').ok_or(InvalidHeader)?;

        if key.trim() != key || key.is_empty() {
            return Err(InvalidHeader);
        }

        let key = key.to_ascii_lowercase();
        let value = value.trim().to_string();
        headers.insert(key, value);
    }

    Ok(headers)
}

#[derive(Debug, PartialEq)]
pub(crate) struct Headers(HashMap<String, String>);

impl Display for StatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatusCode::Ok => write!(f, "200 OK"),
            StatusCode::NotFound => write!(f, "404 Not Found"),
            StatusCode::Found => write!(f, "302 Found"),
            StatusCode::MovedPermanently => write!(f, "301 Moved Permanently"),
            StatusCode::InternalServerError => write!(f, "500 Internal Server Error"),
            StatusCode::BadRequest => write!(f, "400 Bad Request"),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum Method {
    Get,
}

impl std::str::FromStr for Method {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_method(s)
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Response {
    status: StatusCode,
    version: Version,
    headers: Headers,
    body: Vec<u8>,
}

impl Response {
    pub(crate) fn new(status: StatusCode) -> Self {
        Response {
            status,
            version: Version::Http1_1,
            headers: Headers::new(),
            body: vec![],
        }
        .set_header("Connection", "close")
    }

    pub(crate) fn set_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    #[allow(dead_code)]
    pub(crate) fn get_header(&self, key: &str) -> Option<&str> {
        self.headers.get(key)
    }

    pub(crate) fn set_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self.headers
            .insert("Content-Length".to_string(), self.body.len().to_string());
        self
    }

    pub(crate) fn get_status_line(&self) -> String {
        format!("{} {}", self.version, self.status)
    }

    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let mut raw = Vec::new();
        let status_line = self.get_status_line();

        raw.extend_from_slice(status_line.as_bytes());
        raw.extend_from_slice(b"\r\n");

        raw.extend_from_slice(self.headers.to_string().as_bytes());
        raw.extend_from_slice(b"\r\n");

        if !&self.body.is_empty() {
            raw.extend_from_slice(&self.body);
        }

        raw
    }

    #[allow(dead_code)]
    pub(crate) fn status(&self) -> StatusCode {
        self.status
    }

    #[allow(dead_code)]
    pub(crate) fn body(&self) -> &[u8] {
        &self.body
    }
}

impl Default for Headers {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for Headers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (key, value) in &self.0 {
            write!(f, "{}: {}\r\n", key, value)?;
        }
        Ok(())
    }
}

pub(crate) fn parse_method(method: &str) -> Result<Method, Error> {
    match method {
        "GET" => Ok(Method::Get),
        _ => Err(InvalidMethod),
    }
}

pub(crate) fn parse_version(protocol: &str) -> Result<Version, Error> {
    match protocol {
        "HTTP/1.1" => Ok(Version::Http1_1),
        _ => Err(InvalidVersion),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_request() {
        let raw_request = "GET /favicon.ico HTTP/1.1\r\nHost: localhost:4000\r\nConnection: keep-alive\r\nsec-ch-ua-platform: \"macOS\"\r\nUser-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36\r\nsec-ch-ua: \"Not/A)Brand\";v=\"99\", \"Chromium\";v=\"148\"\r\nsec-ch-ua-mobile: ?0\r\nAccept: image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8\r\nSec-Fetch-Site: same-origin\r\nSec-Fetch-Mode: no-cors\r\nSec-Fetch-Dest: image\r\nReferer: http://localhost:4000/nice\r\nAccept-Encoding: gzip, deflate, br, zstd\r\nAccept-Language: en-US,en;q=0.9,ko;q=0.8\r\n\r\nst: document\r\nAccept-Encoding: gzip, deflate, br, zstd\r\nAccept-Language: en-US,en;q=0.9,ko;q=0.8\r\n\r\n\0";

        let request = parse_request(raw_request);
        assert!(request.is_ok());
        let request = request.unwrap();
        assert_eq!(request.method, Method::Get);
        assert_eq!(request.target, "/favicon.ico");
        assert_eq!(request.version, Version::Http1_1);
        assert_eq!(request.body, vec![]);
        assert_eq!(request.headers.get("host").unwrap(), "localhost:4000");
    }

    #[test]
    fn test_parse_request_headers() {
        let mut lines = ["Host: localhost", "Connection: close"].into_iter();
        let headers = parse_request_headers(&mut lines).unwrap();
        assert_eq!(headers.get("host").unwrap(), "localhost");
        assert_eq!(headers.get("connection").unwrap(), "close");
    }

    #[test]
    fn test_parse_request_line() {
        let request = "GET /favicon.ico HTTP/1.1";
        let (method, target, version) = parse_request_line(request).unwrap();
        assert_eq!(method, Method::Get);
        assert_eq!(target, "/favicon.ico");
        assert_eq!(version, Version::Http1_1);
    }

    #[test]
    fn test_http_response_structure() {
        let response = Response::new(StatusCode::Ok)
            .set_header("Content-Type", "text/plain")
            .set_body("hello");

        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.version, Version::Http1_1);
        assert_eq!(response.body, b"hello".to_vec());
        assert_eq!(response.headers.get("Connection"), Some("close"));
        assert_eq!(response.headers.get("Content-Type"), Some("text/plain"));
        assert_eq!(response.headers.get("Content-Length"), Some("5"));
    }

    #[test]
    fn test_http_status_code_reason_phrases() {
        assert_eq!(StatusCode::Ok.to_string(), "200 OK");
        assert_eq!(
            StatusCode::MovedPermanently.to_string(),
            "301 Moved Permanently"
        );
        assert_eq!(StatusCode::Found.to_string(), "302 Found");
        assert_eq!(StatusCode::NotFound.to_string(), "404 Not Found");
        assert_eq!(StatusCode::BadRequest.to_string(), "400 Bad Request");
        assert_eq!(
            StatusCode::InternalServerError.to_string(),
            "500 Internal Server Error"
        );
    }

    #[test]
    fn test_http_response_wire_format() {
        let response = Response::new(StatusCode::Ok)
            .set_header("content-type", "text/plain")
            .set_body("hello");

        assert_eq!(response.status(), StatusCode::Ok);

        let response = String::from_utf8(response.to_bytes()).unwrap();

        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("connection: close\r\n"));
        assert!(response.contains("content-type: text/plain\r\n"));
        assert!(response.contains("content-length: 5\r\n"));
        assert!(response.ends_with("\r\n\r\nhello"));
    }

    #[test]
    fn test_http_response_wire_format_without_body() {
        let response = Response::new(StatusCode::Found).set_header("Location", "/README");

        let response = String::from_utf8(response.to_bytes()).unwrap();

        assert!(response.starts_with("HTTP/1.1 302 Found\r\n"));
        assert!(response.contains("connection: close\r\n"));
        assert!(response.contains("location: /README\r\n"));
        assert!(response.ends_with("\r\n\r\n"));
    }
}
