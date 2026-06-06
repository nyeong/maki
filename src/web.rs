//! Web module
//!
//! ```text
//! http::Request -> Maki -> http::Response
//! ```

use std::io::{Read, Write};
use std::net::TcpListener;

use crate::{HomeMode, Maki, MakiRoute, RunError, http};

const MAX_REQUEST_HEAD_SIZE: usize = 16 * 1024;
const ADDRESS: &str = "127.0.0.1";
const PORT: u16 = 4000;

#[derive(Debug)]
enum Error {
    #[allow(dead_code)]
    Io {
        source: std::io::Error,
    },
    TooLongRequest,
    ZeroLengthRequest,
    #[allow(dead_code)]
    Http(http::Error),
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io { source: error }
    }
}

impl From<http::Error> for Error {
    fn from(error: http::Error) -> Self {
        Self::Http(error)
    }
}

pub(crate) fn handle_request(
    maki: &Maki,
    request: &http::Request,
) -> Result<http::Response, RunError> {
    match maki.resolve_route(request.target()) {
        Ok(MakiRoute::NotePage(path)) => Ok(http::Response::new(http::StatusCode::Ok)
            .set_header("Content-Type", "text/html; charset=utf-8")
            .set_body(maki.render_html(&path)?)),
        Ok(MakiRoute::NoteSource(path)) => Ok(http::Response::new(http::StatusCode::Ok)
            .set_header("Content-Type", "text/plain; charset=utf-8")
            .set_body(maki.get_raw_content(&path)?)),
        Ok(MakiRoute::Home) => match &maki.config.home_mode {
            HomeMode::Redirect(path) => Ok(http::Response::new(http::StatusCode::Found)
                .set_header("Location", path)
                .set_header("Content-Type", "text/plain; charset=utf-8")
                .set_body(path.as_bytes())),
        },
        Ok(MakiRoute::NotFound) => Ok(http::Response::new(http::StatusCode::NotFound)
            .set_header("Content-Type", "text/plain; charset=utf-8")
            .set_body("Not Found".to_string())),
        Err(RunError::BadRequest) => Ok(http::Response::new(http::StatusCode::BadRequest)
            .set_header("Content-Type", "text/plain; charset=utf-8")
            .set_body("Bad Request".to_string())),
        Err(e) => Ok(http::Response::new(http::StatusCode::InternalServerError)
            .set_header("Content-Type", "text/plain; charset=utf-8")
            .set_body(e.to_string())),
    }
}

fn read_request_head(stream: &mut impl Read) -> Result<Vec<u8>, Error> {
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

fn read_request(stream: &mut impl Read) -> Result<http::Request, Error> {
    let raw_request = read_request_head(stream)?;
    // TODO: header만 잘라서 먼저 utf8로 변환하기
    let request = String::from_utf8_lossy(&raw_request);
    let request = http::parse_request(&request)?;
    Ok(request)
}

fn create_response_from_connection(maki: &Maki, stream: &mut impl Read) -> http::Response {
    let request = match read_request(stream) {
        Ok(request) => request,
        Err(err) => {
            eprintln!("Request Parse Error: {:?}", err);
            return http::Response::new(http::StatusCode::BadRequest)
                .set_header("Content-Type", "text/plain; charset=utf-8")
                .set_body("Bad Request");
        }
    };

    match handle_request(maki, &request) {
        Ok(response) => response,
        Err(err) => {
            eprintln!("Response Error: {:?}", err);
            http::Response::new(http::StatusCode::InternalServerError)
                .set_header("Content-Type", "text/plain; charset=utf-8")
                .set_body("Internal Server Error")
        }
    }
}

fn handle_connection<S>(maki: &Maki, stream: &mut S) -> Result<(), RunError>
where
    S: Write + Read,
{
    let response = create_response_from_connection(maki, stream);

    stream
        .write_all(&response.to_bytes())
        .map_err(|source| RunError::IoError { source })
}

pub(crate) fn serve(maki: &Maki) -> Result<(), RunError> {
    let listener =
        TcpListener::bind((ADDRESS, PORT)).map_err(|source| RunError::IoError { source })?;

    println!("Listening on http://{}:{}", ADDRESS, PORT);

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(source) => {
                eprintln!("Failed to accept connection: {}", source);
                continue;
            }
        };

        if let Err(error) = handle_connection(maki, &mut stream) {
            eprintln!("Failed to handle connection: {}", error);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{MakiConfig, web::*};

    #[test]
    fn test_render_not_found_response() {
        let response = http::Response::new(http::StatusCode::NotFound)
            .set_header("Content-Type", "text/plain; charset=utf-8")
            .set_body("Not Found".to_string());
        assert_eq!(response.status(), http::StatusCode::NotFound);
        assert_eq!(response.body(), b"Not Found");
        assert_eq!(
            response.get_header("Content-Type"),
            Some("text/plain; charset=utf-8")
        );
    }

    #[test]
    fn test_read_request_with_split_header() {
        let mut input = &b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n"[..];
        let raw = read_request_head(&mut input).unwrap();
        assert!(raw.ends_with(b"\r\n\r\n"));
    }

    #[test]
    fn test_handle_unknown_path_returns_not_found() {
        let request = http::Request::get("/missing");

        let maki = Maki {
            root: PathBuf::from("."),
            files: vec![],
            config: MakiConfig::default(),
        };

        let response = handle_request(&maki, &request).unwrap();

        assert_eq!(response.status(), http::StatusCode::NotFound);
    }

    #[test]
    fn test_malformed_request_returns_bad_request() {
        let maki = Maki::load(
            &PathBuf::from("./tests/fixtures/basic-maki-project"),
            MakiConfig::default(),
        )
        .unwrap();

        let mut input = &b"GET\r\n\r\n"[..];

        let response = create_response_from_connection(&maki, &mut input);

        assert_eq!(response.status(), http::StatusCode::BadRequest);
        assert_eq!(response.body(), b"Bad Request");
    }

    #[test]
    fn test_empty_request() {
        let mut input = &b""[..];

        assert!(matches!(
            read_request_head(&mut input),
            Err(Error::ZeroLengthRequest)
        ))
    }

    #[test]
    fn test_too_long_request() {
        let bytes = vec![b'a'; MAX_REQUEST_HEAD_SIZE + 1];
        let mut input = &bytes[..];

        assert!(matches!(
            read_request_head(&mut input),
            Err(Error::TooLongRequest)
        ))
    }

    #[test]
    fn test_handle_request() {
        let maki = Maki::load(
            &PathBuf::from("./tests/fixtures/basic-maki-project"),
            MakiConfig::default(),
        )
        .unwrap();

        let request = http::Request::get("/n/daily.md");
        let response = handle_request(&maki, &request).unwrap();
        assert_eq!(response.status(), http::StatusCode::Ok);
        assert!(
            String::from_utf8(response.body().to_vec())
                .unwrap()
                .contains("# Daily")
        );
        assert!(
            response
                .get_header("Content-Type")
                .is_some_and(|v| v.contains("plain"))
        );

        let request = http::Request::get("/n/ignore.txt");
        let response = handle_request(&maki, &request).unwrap();
        assert_eq!(response.status(), http::StatusCode::NotFound);

        let request = http::Request::get("/n/README");
        let response = handle_request(&maki, &request).unwrap();
        assert_eq!(response.status(), http::StatusCode::Ok);
        assert!(
            String::from_utf8(response.body().to_vec())
                .unwrap()
                .contains("Maki")
        );
        assert!(
            response
                .get_header("Content-Type")
                .is_some_and(|v| v.contains("html"))
        );
    }
}
