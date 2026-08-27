use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Instant;

use crate::RunError;
use crate::http::{self, Response};
use crate::metrics::Metrics;
use maki_core::Error as MakiError;

use super::error::{Error, not_found};
use super::live_reload::handle_live_reload_connection;
use super::request_pool::RequestPool;
use super::routes::{response_for_request, route_label_for_request};
use super::state::AppState;
use super::{LIVE_RELOAD_PATH, MAX_REQUEST_HEAD_SIZE, MetricsEndpoint, REQUEST_WORKER_COUNT};

pub(super) fn read_request_head(stream: &mut impl Read) -> Result<Vec<u8>, Error> {
    // TODO: 최적화 가능
    // 매 요청마다 버퍼, Vec 새로 만들지 않고 만들어진 것 쓰기
    // 단, keep-alive 지원할 경우, 그에 대해 고려해야함
    let mut request = Vec::with_capacity(4096);
    let mut buffer = [0u8; 1024];
    loop {
        let bytes_read = stream.read(&mut buffer)?;

        if bytes_read == 0 {
            return Err(Error::ZeroLengthRequest);
        }

        request.extend_from_slice(&buffer[..bytes_read]);

        // TODO: 헤더 경계 찾기 최적화 가능
        // 전체를 훑지 말고 최근에 받은 내용 중에서 훑기
        // buffer만 보면 안됨. \r\n | \r\n 이렇게 끊어서 올 수도 있으니까.
        if request.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(request);
        }

        if request.len() > MAX_REQUEST_HEAD_SIZE {
            return Err(Error::TooLongRequest);
        }
    }
}

pub(super) fn read_request(stream: &mut impl Read) -> Result<http::Request, Error> {
    let raw_request = read_request_head(stream)?;
    // TODO: header만 잘라서 먼저 utf8로 변환하기
    let request = String::from_utf8_lossy(&raw_request);
    let request = http::parse_request(&request)?;
    Ok(request)
}
pub(super) fn write_response(
    stream: &mut impl Write,
    response: Response,
) -> Result<http::StatusCode, RunError> {
    let status = response.status();
    stream
        .write_all(&response.to_bytes())
        .map_err(|source| RunError::IoError { source })?;
    Ok(status)
}
pub(super) fn handle_connection<S>(state: &AppState, stream: &mut S) -> Result<(), RunError>
where
    S: Write + Read,
{
    let request = match read_request(stream) {
        Ok(request) => request,
        Err(err) => {
            let response = err.into_response();
            return stream
                .write_all(&response.to_bytes())
                .map_err(|source| RunError::IoError { source });
        }
    };

    let method = request.method().as_str();
    let route = route_label_for_request(state, &request);

    if request.method() == http::Method::Get && request.target() == LIVE_RELOAD_PATH {
        return handle_live_reload_connection(state, stream).map(|_| ());
    }

    let started = Instant::now();
    let _inflight = state.metrics().track_http_inflight_request();

    let response = match response_for_request(state, &request) {
        Ok(response) => response,
        Err(err) => err.into_response(),
    };
    let status = response.status().code().to_string();
    let response_bytes = response.body().len();

    let result = stream
        .write_all(&response.to_bytes())
        .map_err(|source| RunError::IoError { source });
    state
        .metrics()
        .record_http_request(method, route, status, started.elapsed(), response_bytes);
    result
}

fn metrics_response(metrics: &Metrics) -> Response {
    Response::new(http::StatusCode::Ok)
        .set_header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
        .set_body(metrics.to_prometheus_text())
}

fn handle_metrics_connection<S>(metrics: &Metrics, stream: &mut S) -> Result<(), RunError>
where
    S: Write + Read,
{
    let request = match read_request(stream) {
        Ok(request) => request,
        Err(err) => {
            let response = err.into_response();
            return stream
                .write_all(&response.to_bytes())
                .map_err(|source| RunError::IoError { source });
        }
    };

    let started = Instant::now();
    let response = if request.method() == http::Method::Get && request.target() == "/metrics" {
        let status = http::StatusCode::Ok.code().to_string();
        metrics.record_metrics_request(request.method().as_str(), status, started.elapsed());
        metrics_response(metrics)
    } else {
        let response = not_found(&Error::Maki {
            source: MakiError::NoteNotFound(PathBuf::from("/metrics")),
        });
        let status = response.status().code().to_string();
        metrics.record_metrics_request(request.method().as_str(), status, started.elapsed());
        response
    };

    stream
        .write_all(&response.to_bytes())
        .map_err(|source| RunError::IoError { source })
}

pub(super) fn spawn_metrics_listener(
    listener: TcpListener,
    metrics: Metrics,
    endpoint: MetricsEndpoint,
) -> std::io::Result<()> {
    let request_pool = RequestPool::new("metrics", REQUEST_WORKER_COUNT)?;

    thread::Builder::new()
        .name("maki-metrics-listener".to_string())
        .spawn(move || {
            println!(
                "Metrics listening on http://{}:{}/metrics",
                endpoint.host, endpoint.port
            );

            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(stream) => stream,
                    Err(source) => {
                        eprintln!("Failed to accept metrics connection: {}", source);
                        continue;
                    }
                };

                let metrics = metrics.clone();
                request_pool.execute(move || {
                    if let Err(error) = handle_metrics_connection(&metrics, &mut stream) {
                        eprintln!("Failed to handle metrics connection: {}", error);
                    }
                });
            }
        })?;

    Ok(())
}
