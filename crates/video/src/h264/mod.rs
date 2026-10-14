#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

mod cabac;
mod cabac_mb;
#[rustfmt::skip]
mod cabac_tables;
mod cavlc;
#[rustfmt::skip]
mod cavlc_tables;
mod deblock;
mod decoder;
mod dpb;
mod error;
mod inter;
mod intra;
mod mbtypes;
pub mod params;
mod picture;
pub use picture::set_plane_allocator;
pub mod slice;
mod slicedec;
mod tables;
mod transform;

pub use decoder::{DecodeStats, Decoder, default_threads};
pub use error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorInfo {
    pub full_range: bool,
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
}

#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub chroma_width: u32,
    pub chroma_height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub y_stride: usize,
    pub uv_stride: usize,
    pub pts: i64,
    pub poc: i32,
    pub key: bool,
    pub color: ColorInfo,
    pub sar: (u16, u16),
    pub draft: bool,
}
