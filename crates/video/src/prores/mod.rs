mod bits;
mod dct;
mod decode;
mod encode;
pub mod header;
mod tables;

pub use decode::{DecodeOptions, decode_frame, decode_frame_into, decode_frame_with, probe};
pub use encode::{Encoder, EncoderConfig};
pub use header::{FrameHeader, PictureHeader};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("truncated ProRes data")]
    Truncated,
    #[error("invalid ProRes data: {0}")]
    Invalid(&'static str),
    #[error("unsupported ProRes feature: {0}")]
    Unsupported(&'static str),
    #[error("corrupt slice {slice} in picture {picture}")]
    Slice { picture: usize, slice: usize },
    #[error("invalid encoder input: {0}")]
    Input(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChromaFormat {
    Yuv422,
    Yuv444,
}

impl ChromaFormat {
    pub fn chroma_width(self, width: u32) -> u32 {
        match self {
            ChromaFormat::Yuv422 => width.div_ceil(2),
            ChromaFormat::Yuv444 => width,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Interlace {
    #[default]
    Progressive,
    TopFieldFirst,
    BottomFieldFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AlphaType {
    #[default]
    None,
    Bits8,
    Bits16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorInfo {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
}

impl Default for ColorInfo {
    fn default() -> Self {
        ColorInfo { primaries: 1, transfer: 1, matrix: 1 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Profile {
    Proxy,
    Lt,
    Standard,
    Hq,
    P4444,
    P4444Xq,
}

impl Profile {
    pub const ALL: [Profile; 6] = [Profile::Proxy, Profile::Lt, Profile::Standard, Profile::Hq, Profile::P4444, Profile::P4444Xq];

    pub fn fourcc(self) -> [u8; 4] {
        *match self {
            Profile::Proxy => b"apco",
            Profile::Lt => b"apcs",
            Profile::Standard => b"apcn",
            Profile::Hq => b"apch",
            Profile::P4444 => b"ap4h",
            Profile::P4444Xq => b"ap4x",
        }
    }

    pub fn from_fourcc(f: &[u8; 4]) -> Option<Profile> {
        Profile::ALL.into_iter().find(|p| &p.fourcc() == f)
    }

    pub fn chroma(self) -> ChromaFormat {
        match self {
            Profile::P4444 | Profile::P4444Xq => ChromaFormat::Yuv444,
            _ => ChromaFormat::Yuv422,
        }
    }

    pub fn nominal_mbps_1080p30(self) -> f64 {
        match self {
            Profile::Proxy => 45.0,
            Profile::Lt => 102.0,
            Profile::Standard => 147.0,
            Profile::Hq => 220.0,
            Profile::P4444 => 330.0,
            Profile::P4444Xq => 500.0,
        }
    }

    pub fn nominal_bits_per_mb(self) -> u32 {
        (self.nominal_mbps_1080p30() * 1e6 * 1001.0 / 30000.0 / 8160.0).round() as u32
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub chroma: ChromaFormat,
    pub bit_depth: u8,
    pub y: Vec<u16>,
    pub cb: Vec<u16>,
    pub cr: Vec<u16>,
    pub alpha: Option<Vec<u16>>,
    pub interlace: Interlace,
    pub color: ColorInfo,
    pub aspect_ratio: u8,
    pub frame_rate_code: u8,
}

impl Frame {
    pub fn new(width: u32, height: u32, chroma: ChromaFormat, bit_depth: u8, with_alpha: bool) -> Frame {
        let n = width as usize * height as usize;
        let cn = chroma.chroma_width(width) as usize * height as usize;
        let s = bit_depth.saturating_sub(8) as u32;
        Frame {
            width,
            height,
            chroma,
            bit_depth,
            y: vec![(16u32 << s) as u16; n],
            cb: vec![(128u32 << s) as u16; cn],
            cr: vec![(128u32 << s) as u16; cn],
            alpha: with_alpha.then(|| vec![((1u32 << bit_depth) - 1) as u16; n]),
            interlace: Interlace::Progressive,
            color: ColorInfo::default(),
            aspect_ratio: 0,
            frame_rate_code: 0,
        }
    }

    pub fn chroma_width(&self) -> u32 {
        self.chroma.chroma_width(self.width)
    }

    pub fn is_interlaced(&self) -> bool {
        self.interlace != Interlace::Progressive
    }
}
