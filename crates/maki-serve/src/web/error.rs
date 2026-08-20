use crate::http::{self, Response};
use maki_core::Error as MakiError;

#[derive(Debug)]
pub(super) enum Error {
    #[allow(dead_code)]
    Io {
        source: std::io::Error,
    },
    InvalidRequest {
        #[allow(dead_code)]
        source: http::Error,
    },
    TooLongRequest,
    ZeroLengthRequest,
    BadRequest,
    Maki {
        source: MakiError,
    },
}

pub(super) fn internal_server_error(e: &Error) -> Response {
    Response::new(http::StatusCode::InternalServerError)
        .set_header("content-type", "text/plain")
        .set_body(format!("Internal Server Error: {}", e))
}

pub(super) fn not_found(e: &Error) -> Response {
    Response::new(http::StatusCode::NotFound)
        .set_header("content-type", "text/plain")
        .set_body(format!("Not Found: {}", e))
}

pub(super) fn bad_request(e: &Error) -> Response {
    Response::new(http::StatusCode::BadRequest)
        .set_header("content-type", "text/plain")
        .set_body(format!("Bad Request: {}", e))
}

impl Error {
    pub(super) fn into_response(self) -> Response {
        match self {
            e @ Error::Maki {
                source: MakiError::NoteNotFound(..),
            } => not_found(&e),
            e @ Error::Maki {
                source: MakiError::InvalidNotePath(..),
            }
            | e @ Error::InvalidRequest { .. }
            | e @ Error::TooLongRequest
            | e @ Error::BadRequest
            | e @ Error::ZeroLengthRequest => bad_request(&e),
            e @ Error::Io { .. } | e @ Error::Maki { .. } => internal_server_error(&e),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io { source: error }
    }
}

impl From<MakiError> for Error {
    fn from(error: MakiError) -> Self {
        Self::Maki { source: error }
    }
}

impl From<http::Error> for Error {
    fn from(error: http::Error) -> Self {
        Self::InvalidRequest { source: error }
    }
}
