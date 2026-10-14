use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("truncated data while reading {0}")]
    Truncated(&'static str),
    #[error("invalid data: {0}")]
    Invalid(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("size limit exceeded: {0}")]
    TooLarge(&'static str),
    #[error("no moov box found")]
    NoMoov,
    #[error("track index {0} out of range")]
    NoSuchTrack(usize),
    #[error("sample index {0} out of range")]
    NoSuchSample(usize),
    #[error("muxer: {0}")]
    Mux(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}
