#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

pub mod bitstream;
pub mod convert;
pub mod h264;
pub mod isobmff;
mod movie;
pub mod prores;
pub mod synth;

pub use isobmff::ByteSource;
pub use movie::{Frame, Movie, MovieInfo, SharedSource, fit_size};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum VideoError {
    #[error("not a readable movie: {0}")]
    Container(String),
    #[error("the movie has no video track")]
    NoVideoTrack,
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("could not decode the picture: {0}")]
    Decode(String),
    #[error("no picture at that time")]
    OutOfRange,
}

pub type Result<T> = std::result::Result<T, VideoError>;

pub fn probe(bytes: &[u8]) -> Result<MovieInfo> {
    Movie::open(bytes.to_vec()).map(|m| m.info().clone())
}

#[cfg(test)]
mod tests;
