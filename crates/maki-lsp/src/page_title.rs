use std::collections::BTreeSet;
use std::io::{self, Read};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

use lsp_types::Url;

const MAX_URL_BYTES: usize = 8 * 1024;
const MAX_REDIRECTS: usize = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_TITLE_CHARS: usize = 160;
const FETCH_QUEUE_CAPACITY: usize = 1;

pub(crate) trait PageTitleProvider: Send + Sync {
    fn page_title(&self, url: &Url) -> Option<String>;
}

#[derive(Clone)]
pub(crate) struct HttpPageTitleProvider {
    worker: FetchWorker,
}

impl HttpPageTitleProvider {
    pub(crate) fn new() -> Self {
        static WORKER: OnceLock<FetchWorker> = OnceLock::new();
        let worker = WORKER
            .get_or_init(|| FetchWorker::spawn(TOTAL_TIMEOUT, fetch_http_title))
            .clone();
        Self { worker }
    }

    #[cfg(test)]
    fn with_fetcher(
        response_timeout: Duration,
        fetch: impl FnMut(&Url, Instant) -> Option<String> + Send + 'static,
    ) -> Self {
        Self {
            worker: FetchWorker::spawn(response_timeout, fetch),
        }
    }
}

#[derive(Clone)]
struct FetchWorker {
    requests: mpsc::SyncSender<FetchRequest>,
    in_flight: Arc<AtomicBool>,
    response_timeout: Duration,
}

struct FetchRequest {
    url: Url,
    deadline: Instant,
    response: mpsc::SyncSender<Option<String>>,
}

impl FetchWorker {
    fn spawn(
        response_timeout: Duration,
        mut fetch: impl FnMut(&Url, Instant) -> Option<String> + Send + 'static,
    ) -> Self {
        let (requests, request_receiver) = mpsc::sync_channel::<FetchRequest>(FETCH_QUEUE_CAPACITY);
        let in_flight = Arc::new(AtomicBool::new(false));
        let worker_in_flight = Arc::clone(&in_flight);

        // DNS lookup through the standard library cannot be cancelled. Keep it
        // off the LSP protocol thread in one process-wide worker. The bounded
        // queue and in-flight guard ensure a timed-out lookup cannot accumulate
        // an unbounded number of jobs or detached threads.
        drop(
            std::thread::Builder::new()
                .name("maki-page-title".to_string())
                .spawn(move || {
                    while let Ok(request) = request_receiver.recv() {
                        worker_in_flight.store(true, Ordering::Release);
                        let result = (Instant::now() < request.deadline)
                            .then(|| fetch(&request.url, request.deadline))
                            .flatten();
                        worker_in_flight.store(false, Ordering::Release);
                        let _ = request.response.try_send(result);
                    }
                }),
        );

        Self {
            requests,
            in_flight,
            response_timeout,
        }
    }

    fn fetch(&self, url: &Url) -> Option<String> {
        if self.in_flight.load(Ordering::Acquire) || !is_fetchable_url(url) {
            return None;
        }

        let deadline = Instant::now().checked_add(self.response_timeout)?;
        let (response, response_receiver) = mpsc::sync_channel(1);
        self.requests
            .try_send(FetchRequest {
                url: url.clone(),
                deadline,
                response,
            })
            .ok()?;
        let remaining = deadline.checked_duration_since(Instant::now())?;
        response_receiver.recv_timeout(remaining).ok().flatten()
    }
}

fn fetch_http_title(requested_url: &Url, deadline: Instant) -> Option<String> {
    let mut current = normalized_request_url(requested_url)?;
    let mut seen = BTreeSet::new();

    for redirect_count in 0..=MAX_REDIRECTS {
        if !seen.insert(current.as_str().to_owned()) {
            return None;
        }

        // Resolve separately so the HTTP client's timeout starts after DNS and
        // uses only the remaining budget. The worker protects the LSP thread if
        // the platform resolver itself blocks past this deadline.
        let addresses = resolve_public_addresses(&current, deadline).ok()?;
        let remaining = deadline.checked_duration_since(Instant::now())?;
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT.min(remaining))
            .timeout(remaining)
            .redirects(0)
            .try_proxy_from_env(false)
            .max_idle_connections(0)
            .user_agent(concat!("maki-lsp/", env!("CARGO_PKG_VERSION")))
            .resolver(PinnedResolver { addresses })
            .build();

        let result = agent
            .get(current.as_str())
            .timeout(remaining)
            .set("Accept", "text/html, application/xhtml+xml;q=0.9")
            .set("Accept-Encoding", "identity")
            .set("Connection", "close")
            .call();
        let response = response_even_for_http_status(result)?;

        if !is_public_ip(response.remote_addr().ip()) {
            return None;
        }

        let status = response.status();
        if is_redirect_status(status) {
            if redirect_count == MAX_REDIRECTS {
                return None;
            }
            let location = response.header("Location")?;
            current = safe_redirect_target(&current, location)?;
            continue;
        }

        if !(200..300).contains(&status)
            || !is_html_content_type(response.header("Content-Type"))
            || !is_identity_content_encoding(response.header("Content-Encoding"))
            || content_length_exceeds_limit(response.header("Content-Length"))
        {
            return None;
        }

        let mut body = Vec::new();
        response
            .into_reader()
            .take((MAX_BODY_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .ok()?;
        if body.len() > MAX_BODY_BYTES {
            return None;
        }
        return extract_html_title(&body);
    }

    None
}

impl Default for HttpPageTitleProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl PageTitleProvider for HttpPageTitleProvider {
    fn page_title(&self, url: &Url) -> Option<String> {
        self.worker.fetch(url)
    }
}

fn response_even_for_http_status(
    result: Result<ureq::Response, ureq::Error>,
) -> Option<ureq::Response> {
    match result {
        Ok(response) | Err(ureq::Error::Status(_, response)) => Some(response),
        Err(ureq::Error::Transport(_)) => None,
    }
}

fn normalized_request_url(url: &Url) -> Option<Url> {
    is_fetchable_url(url).then(|| {
        let mut normalized = url.clone();
        normalized.set_fragment(None);
        normalized
    })
}

pub(crate) fn is_fetchable_url(url: &Url) -> bool {
    if url.as_str().len() > MAX_URL_BYTES
        || !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return false;
    }

    let Some(raw_host) = url.host_str() else {
        return false;
    };
    let host = raw_host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(raw_host);
    if host.is_empty() || host.contains('%') || is_local_hostname(host) {
        return false;
    }

    match host.parse::<IpAddr>() {
        Ok(ip) => is_public_ip(ip),
        Err(_) => true,
    }
}

fn is_local_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    [
        "localhost",
        "local",
        "internal",
        "home.arpa",
        "localdomain",
        "test",
        "invalid",
        "example",
    ]
    .iter()
    .any(|suffix| {
        host == *suffix
            || host
                .strip_suffix(suffix)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

fn is_redirect_status(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

pub(crate) fn safe_redirect_target(current: &Url, location: &str) -> Option<Url> {
    let location = location.trim();
    if location.is_empty() {
        return None;
    }

    let next = current.join(location).ok()?;
    if current.scheme() == "https" && next.scheme() != "https" {
        return None;
    }
    normalized_request_url(&next)
}

fn is_html_content_type(content_type: Option<&str>) -> bool {
    let Some(media_type) = content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
    else {
        return false;
    };

    media_type.eq_ignore_ascii_case("text/html")
        || media_type.eq_ignore_ascii_case("application/xhtml+xml")
}

fn is_identity_content_encoding(content_encoding: Option<&str>) -> bool {
    content_encoding.is_none_or(|encoding| encoding.trim().eq_ignore_ascii_case("identity"))
}

fn content_length_exceeds_limit(content_length: Option<&str>) -> bool {
    content_length.is_some_and(|length| {
        length
            .trim()
            .parse::<u64>()
            .map_or(true, |length| length > MAX_BODY_BYTES as u64)
    })
}

#[derive(Debug, Clone)]
struct PinnedResolver {
    addresses: Vec<SocketAddr>,
}

impl ureq::Resolver for PinnedResolver {
    fn resolve(&self, _netloc: &str) -> io::Result<Vec<SocketAddr>> {
        Ok(self.addresses.clone())
    }
}

fn resolve_public_addresses(url: &Url, deadline: Instant) -> io::Result<Vec<SocketAddr>> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "page-title DNS deadline elapsed",
        ));
    }

    let addresses = url.socket_addrs(|| None)?;
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "page-title DNS deadline elapsed",
        ));
    }

    let public = public_socket_addrs(addresses);
    if public.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "destination has no public IP address",
        ))
    } else {
        Ok(public)
    }
}

pub(crate) fn public_socket_addrs(
    addresses: impl IntoIterator<Item = SocketAddr>,
) -> Vec<SocketAddr> {
    addresses
        .into_iter()
        .filter(|address| is_public_ip(address.ip()))
        .collect()
}

pub(crate) fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();

    !(a == 0
        || a == 10
        || (a == 100 && (64..=127).contains(&b))
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && matches!(b, 18 | 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();

    // Public unicast is currently allocated from 2000::/3. Conservatively reject
    // protocol, documentation, transition, and formerly allocated ranges within it.
    (segments[0] & 0xe000) == 0x2000
        && !(segments[0] == 0x2001 && segments[1] <= 0x01ff)
        && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        && segments[0] != 0x2002
        && segments[0] != 0x3ffe
        && (segments[0] & 0xfff0) != 0x3ff0
}

pub(crate) fn extract_html_title(body: &[u8]) -> Option<String> {
    if body.len() > MAX_BODY_BYTES {
        return None;
    }
    let html = std::str::from_utf8(body).ok()?;
    let raw_title = find_title_text(html)?;
    sanitize_title(&decode_html_entities(raw_title))
}

fn find_title_text(html: &str) -> Option<&str> {
    let bytes = html.as_bytes();
    let mut cursor = 0;

    while let Some(relative_start) = bytes[cursor..].iter().position(|byte| *byte == b'<') {
        let start = cursor + relative_start;

        if bytes[start..].starts_with(b"<!--") {
            let comment_end = find_bytes(bytes, start + 4, b"-->")?;
            cursor = comment_end + 3;
            continue;
        }

        if tag_at(bytes, start, b"head", true) {
            return None;
        }

        if tag_at(bytes, start, b"title", false) {
            let open_end = find_tag_end(bytes, start + 1 + b"title".len())?;
            let (close_start, _) = find_end_tag(bytes, open_end + 1, b"title")?;
            return html.get(open_end + 1..close_start);
        }

        if tag_at(bytes, start, b"script", false) || tag_at(bytes, start, b"style", false) {
            let name: &[u8] = if tag_at(bytes, start, b"script", false) {
                b"script"
            } else {
                b"style"
            };
            let open_end = find_tag_end(bytes, start + 1 + name.len())?;
            let (_, close_end) = find_end_tag(bytes, open_end + 1, name)?;
            cursor = close_end + 1;
            continue;
        }

        // A malformed tag without `>` consumes the remainder. Retrying at each
        // nested `<` would rescan the same suffix and make hostile input
        // quadratic; a valid tag advances past its closing delimiter.
        cursor = find_tag_end(bytes, start + 1)? + 1;
    }

    None
}

fn tag_at(bytes: &[u8], start: usize, name: &[u8], closing: bool) -> bool {
    let prefix_len = 1 + usize::from(closing);
    let Some(prefix) = bytes.get(start..start + prefix_len) else {
        return false;
    };
    if prefix.first() != Some(&b'<') || (closing && prefix.get(1) != Some(&b'/')) {
        return false;
    }

    let name_start = start + prefix_len;
    let Some(candidate) = bytes.get(name_start..name_start + name.len()) else {
        return false;
    };
    candidate.eq_ignore_ascii_case(name)
        && bytes
            .get(name_start + name.len())
            .is_none_or(|byte| byte.is_ascii_whitespace() || matches!(byte, b'>' | b'/'))
}

fn find_tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut quote = None;

    for (index, byte) in bytes.iter().copied().enumerate().skip(start) {
        match (quote, byte) {
            (Some(expected), current) if current == expected => quote = None,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'>') => return Some(index),
            _ => {}
        }
    }

    None
}

fn find_end_tag(bytes: &[u8], mut cursor: usize, name: &[u8]) -> Option<(usize, usize)> {
    while let Some(relative_start) = bytes[cursor..].iter().position(|byte| *byte == b'<') {
        let start = cursor + relative_start;
        if tag_at(bytes, start, name, true) {
            return Some((start, find_tag_end(bytes, start + name.len() + 2)?));
        }
        cursor = start + 1;
    }

    None
}

fn find_bytes(bytes: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(start..)?
        .windows(needle.len())
        .position(|candidate| candidate == needle)
        .map(|relative| start + relative)
}

fn decode_html_entities(input: &str) -> String {
    let mut decoded = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(relative_ampersand) = input[cursor..].find('&') {
        let ampersand = cursor + relative_ampersand;
        decoded.push_str(&input[cursor..ampersand]);

        let candidate_start = ampersand + 1;
        let Some(relative_semicolon) = input[candidate_start..]
            .get(..32.min(input.len() - candidate_start))
            .and_then(|candidate| candidate.find(';'))
        else {
            decoded.push('&');
            cursor = candidate_start;
            continue;
        };
        let semicolon = candidate_start + relative_semicolon;
        let entity = &input[candidate_start..semicolon];

        if let Some(character) = decode_html_entity(entity) {
            decoded.push(character);
            cursor = semicolon + 1;
        } else {
            decoded.push('&');
            cursor = candidate_start;
        }
    }

    decoded.push_str(&input[cursor..]);
    decoded
}

fn decode_html_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{00a0}'),
        _ => {
            let value = if let Some(hex) = entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
            {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                entity.strip_prefix('#')?.parse().ok()?
            };
            char::from_u32(value)
        }
    }
}

fn sanitize_title(title: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut pending_space = false;
    let mut character_count = 0;

    for character in title.chars() {
        if character.is_whitespace() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if character.is_control() || is_bidi_control(character) || character == '\u{feff}' {
            continue;
        }

        if pending_space {
            if character_count == MAX_TITLE_CHARS {
                break;
            }
            normalized.push(' ');
            character_count += 1;
            pending_space = false;
        }
        if character_count == MAX_TITLE_CHARS {
            break;
        }

        normalized.push(match character {
            '[' => '(',
            ']' => ')',
            // A raw vertical bar ends a Maki table cell. Use the visually
            // equivalent full-width form so a fetched title remains safe when
            // the URL occurrence is inside a table.
            '|' => '｜',
            _ => character,
        });
        character_count += 1;
    }

    (!normalized.is_empty() && !normalized.starts_with('^')).then_some(normalized)
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    #[test]
    fn url_policy_accepts_only_public_http_urls_with_default_ports() {
        for allowed in [
            "https://example.com/",
            "http://example.com/path?q=value#fragment",
            "https://example.com:443/",
            "http://example.com:80/",
            "https://1.1.1.1/",
            "https://[2606:4700:4700::1111]/",
        ] {
            assert!(is_fetchable_url(&url(allowed)), "should allow {allowed}");
        }

        for denied in [
            "ftp://example.com/",
            "file:///tmp/page.html",
            "https://user@example.com/",
            "https://user:password@example.com/",
            "https://example.com:8443/",
            "http://example.com:8080/",
            "http://localhost/",
            "http://service.local/",
            "http://metadata.google.internal/",
            "http://router.home.arpa/",
            "http://example.test/",
            "http://127.0.0.1/",
            "http://127.1/",
            "http://2130706433/",
            "http://0x7f000001/",
            "http://0177.0.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[fc00::1]/",
        ] {
            assert!(!is_fetchable_url(&url(denied)), "should deny {denied}");
        }

        let oversized = format!("https://example.com/{}", "a".repeat(MAX_URL_BYTES));
        assert!(!is_fetchable_url(&url(&oversized)));
    }

    #[test]
    fn worker_bounds_protocol_wait_and_rejects_more_work_while_lookup_is_stuck() {
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = Arc::clone(&calls);
        let (started_sender, started_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let (finished_sender, finished_receiver) = mpsc::channel();
        let response_timeout = Duration::from_millis(250);
        let provider =
            HttpPageTitleProvider::with_fetcher(response_timeout, move |_url, _deadline| {
                worker_calls.fetch_add(1, Ordering::SeqCst);
                started_sender.send(()).ok()?;
                release_receiver.recv().ok()?;
                finished_sender.send(()).ok()?;
                Some("too late".to_string())
            });

        let caller_provider = provider.clone();
        let (result_sender, result_receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = caller_provider.page_title(&url("https://1.1.1.1/"));
            let _ = result_sender.send((result, started.elapsed()));
        });

        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("the dedicated fetch worker should start");
        let (result, elapsed) = result_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("the LSP-facing wait must remain bounded");
        assert_eq!(result, None);
        assert!(elapsed < Duration::from_secs(1));
        assert!(provider.worker.in_flight.load(Ordering::Acquire));

        let second_started = Instant::now();
        assert_eq!(provider.page_title(&url("https://1.0.0.1/")), None);
        assert!(second_started.elapsed() < response_timeout);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Closing the only request sender before releasing the fake resolver
        // lets the fixed worker terminate instead of leaving a test thread.
        drop(provider);
        release_sender.send(()).unwrap();
        finished_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("the fake blocking lookup should be released");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn redirects_are_reparsed_and_rechecked() {
        let https = url("https://example.com/one/page");
        assert_eq!(
            safe_redirect_target(&https, "../next#part")
                .unwrap()
                .as_str(),
            "https://example.com/next"
        );
        assert_eq!(
            safe_redirect_target(&url("http://example.com/"), "https://example.org/")
                .unwrap()
                .as_str(),
            "https://example.org/"
        );

        for denied in [
            "http://example.com/",
            "https://127.0.0.1/",
            "https://user@example.org/",
            "https://example.org:8443/",
            "file:///tmp/page.html",
            "   ",
        ] {
            assert!(
                safe_redirect_target(&https, denied).is_none(),
                "should deny redirect to {denied:?}"
            );
        }
    }

    #[test]
    fn ipv4_policy_rejects_non_public_ranges_and_their_boundaries() {
        for denied in [
            "0.0.0.0",
            "0.255.255.255",
            "10.0.0.1",
            "100.64.0.0",
            "100.127.255.255",
            "127.0.0.1",
            "169.254.0.1",
            "172.16.0.0",
            "172.31.255.255",
            "192.0.0.1",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.0.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "239.255.255.255",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!is_public_ip(ip(denied)), "should deny {denied}");
        }

        for allowed in [
            "1.1.1.1",
            "9.255.255.255",
            "11.0.0.0",
            "100.63.255.255",
            "100.128.0.0",
            "126.255.255.255",
            "128.0.0.0",
            "169.253.255.255",
            "169.255.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "192.169.0.1",
            "198.17.255.255",
            "198.20.0.0",
            "203.0.114.0",
            "223.255.255.254",
        ] {
            assert!(is_public_ip(ip(allowed)), "should allow {allowed}");
        }
    }

    #[test]
    fn ipv6_policy_is_conservative_about_special_and_transition_ranges() {
        for denied in [
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "64:ff9b::127.0.0.1",
            "100::1",
            "1fff::1",
            "2001::1",
            "2001:1ff::1",
            "2001:db8::1",
            "2002::1",
            "3ffe::1",
            "3fff::1",
            "4000::1",
            "fc00::1",
            "fdff::1",
            "fe80::1",
            "febf::1",
            "ff02::1",
        ] {
            assert!(!is_public_ip(ip(denied)), "should deny {denied}");
        }

        for allowed in [
            "2001:200::1",
            "2001:4860:4860::8888",
            "2606:4700:4700::1111",
            "2a00:1450:4001::1",
            "3fef:ffff::1",
        ] {
            assert!(is_public_ip(ip(allowed)), "should allow {allowed}");
        }
    }

    #[test]
    fn resolver_filter_returns_only_pinned_public_addresses() {
        let public_v4: SocketAddr = "1.1.1.1:443".parse().unwrap();
        let public_v6: SocketAddr = "[2606:4700:4700::1111]:443".parse().unwrap();
        let addresses = [
            "127.0.0.1:443".parse().unwrap(),
            public_v4,
            "[::1]:443".parse().unwrap(),
            public_v6,
            "169.254.169.254:80".parse().unwrap(),
        ];

        assert_eq!(public_socket_addrs(addresses), vec![public_v4, public_v6]);
    }

    #[test]
    fn title_extraction_handles_case_attributes_whitespace_and_entities() {
        let html = br#"<!doctype html><HEAD><TITLE data-kind='page'>
              Maki &amp; Friends &#x1f363; &#8212;&nbsp;Docs
            </TiTlE></HEAD>"#;

        assert_eq!(
            extract_html_title(html).as_deref(),
            Some("Maki & Friends 🍣 — Docs")
        );
        assert_eq!(
            extract_html_title(b"<title>A &bogus; B</title>").as_deref(),
            Some("A &bogus; B"),
            "unknown entities should be preserved exactly once"
        );
    }

    #[test]
    fn title_sanitization_neutralizes_table_cell_delimiters() {
        assert_eq!(
            extract_html_title(b"<title>Left | Right</title>").as_deref(),
            Some("Left ｜ Right")
        );
    }

    #[test]
    fn title_extraction_ignores_comments_scripts_styles_and_title_like_names() {
        let html = br#"
            <!-- <title>comment</title> -->
            <script>const sample = '<title>script</title>';</script>
            <style>/* <title>style</title> */</style>
            <title-card>not a title</title-card>
            <title>Actual title</title>
        "#;

        assert_eq!(extract_html_title(html).as_deref(), Some("Actual title"));
    }

    #[test]
    fn title_extraction_rejects_invalid_or_missing_titles() {
        for body in [
            b"<html><head></head><body>none</body></html>".as_slice(),
            b"<title>unfinished".as_slice(),
            b"<title>   \n\t </title>".as_slice(),
            b"</head><body><title>too late</title></body>".as_slice(),
            b"<title>^reserved</title>".as_slice(),
            b"<title>\xff</title>".as_slice(),
        ] {
            assert!(extract_html_title(body).is_none(), "body was {body:?}");
        }
    }

    #[test]
    fn title_sanitization_neutralizes_maki_and_bidi_injection() {
        let body = "<title>  Safe\n[key]: <evil> \u{202e}tail\u{2066}\u{0000} </title>";

        assert_eq!(
            extract_html_title(body.as_bytes()).as_deref(),
            Some("Safe (key): <evil> tail")
        );
    }

    #[test]
    fn title_sanitization_caps_unicode_scalar_count() {
        let body = format!("<title>{}</title>", "🍣".repeat(MAX_TITLE_CHARS + 5));
        let title = extract_html_title(body.as_bytes()).unwrap();

        assert_eq!(title.chars().count(), MAX_TITLE_CHARS);
        assert!(title.chars().all(|character| character == '🍣'));
    }

    #[test]
    fn title_extraction_rejects_a_body_over_the_byte_limit() {
        let mut body = b"<title>Early title</title>".to_vec();
        body.resize(MAX_BODY_BYTES + 1, b' ');

        assert!(extract_html_title(&body).is_none());
    }

    #[test]
    fn title_scan_is_linear_for_a_maximum_size_unclosed_tag_run() {
        let body = vec![b'<'; MAX_BODY_BYTES];

        assert_eq!(extract_html_title(&body), None);
    }

    #[test]
    fn response_metadata_policy_requires_unencoded_html() {
        for accepted in [
            Some("text/html"),
            Some("TEXT/HTML; charset=UTF-8"),
            Some("application/xhtml+xml; charset=utf-8"),
        ] {
            assert!(is_html_content_type(accepted));
        }
        for denied in [None, Some("text/plain"), Some("application/xml")] {
            assert!(!is_html_content_type(denied));
        }

        assert!(is_identity_content_encoding(None));
        assert!(is_identity_content_encoding(Some(" Identity ")));
        assert!(!is_identity_content_encoding(Some("gzip")));
        assert!(!is_identity_content_encoding(Some("br, identity")));

        assert!(!content_length_exceeds_limit(None));
        assert!(!content_length_exceeds_limit(Some("262144")));
        assert!(content_length_exceeds_limit(Some("262145")));
        assert!(content_length_exceeds_limit(Some("invalid")));
    }
}
