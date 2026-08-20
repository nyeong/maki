use std::fmt::Display;

pub mod git_source;
pub mod metrics;
pub mod web;

pub use maki_http as http;

#[derive(Debug)]
pub enum RunError {
    IoError { source: std::io::Error },
}

impl Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IoError { source } => write!(f, "IO error: {}", source),
        }
    }
}
