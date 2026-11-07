#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod aiff;
mod buffer;
mod caf;
mod field;
pub use field::{FieldInfo, field_info};
mod flac;
#[cfg(not(target_arch = "wasm32"))]
mod mp3;
#[cfg(target_arch = "wasm32")]
mod mp3 {
    pub fn encode(_: &crate::AudioBuffer) -> crate::Result<Vec<u8>> {
        Err(crate::AudioError::Encode("MP3 export is not available in the browser".into()))
    }
}
mod mp4;
mod pcm;
pub mod peaks;
mod symph;
mod wav;

pub use buffer::AudioBuffer;
use std::io::{Seek, Write};

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("unsupported audio: {0}")]
    Unsupported(String),
    #[error("malformed audio: {0}")]
    Malformed(String),
    #[error("audio too large: {0}")]
    TooLarge(String),
    #[error("audio encode failed: {0}")]
    Encode(String),
    #[error("audio write failed: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, AudioError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FileFormat {
    Wav,
    Aiff,
    Flac,
    Mp3,
    Ogg,
    Aac,
    Alac,
    Caf,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SampleFormat {
    Int8,
    Int16,
    Int24,
    Int32,
    Float32,
    Float64,
    Compressed,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BwfInfo {
    pub description: String,
    pub originator: String,
    pub originator_reference: String,
    pub origination_date: String,
    pub origination_time: String,
    pub time_reference: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AudioInfo {
    pub format: FileFormat,
    pub sample_format: SampleFormat,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub bwf: Option<BwfInfo>,
}

impl AudioInfo {
    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 { 0.0 } else { self.frames as f64 / f64::from(self.sample_rate) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BitDepth {
    Int16,
    Int24,
    Int32,
    Float32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EncodeOptions {
    pub format: FileFormat,
    pub bit_depth: BitDepth,
    pub dither: bool,
    pub bwf: Option<BwfInfo>,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self { format: FileFormat::Wav, bit_depth: BitDepth::Int24, dither: false, bwf: None }
    }
}

fn id3v2_len(b: &[u8]) -> Option<usize> {
    if b.get(..3)? != b"ID3" {
        return None;
    }
    let flags = *b.get(5)?;
    let size = b.get(6..10)?.iter().fold(0usize, |acc, &x| (acc << 7) | usize::from(x & 0x7F));
    Some(10 + size + if flags & 0x10 != 0 { 10 } else { 0 })
}

fn detect_by_magic(b: &[u8], depth: u32) -> Option<FileFormat> {
    let head = |n: usize| b.get(..n);
    if wav::is_wav(b) {
        return Some(FileFormat::Wav);
    }
    if aiff::is_aiff(b) {
        return Some(FileFormat::Aiff);
    }
    match head(4)? {
        b"fLaC" => return Some(FileFormat::Flac),
        b"OggS" => return Some(FileFormat::Ogg),
        b"caff" => return Some(FileFormat::Caf),
        [0x1A, 0x45, 0xDF, 0xA3] => return Some(FileFormat::Other),
        _ => {}
    }
    if b.get(4..8) == Some(b"ftyp") {
        let found = b.windows(4).position(|w| w == b"stsd");
        let codec = found.and_then(|p| b.get(p.checked_add(16)?..p.checked_add(20)?));
        return Some(if codec == Some(b"alac") { FileFormat::Alac } else { FileFormat::Aac });
    }
    if let Some(len) = id3v2_len(b) {
        if depth == 0
            && let Some(f) = b.get(len..).and_then(|rest| detect_by_magic(rest, 1))
        {
            return Some(f);
        }
        return Some(FileFormat::Mp3);
    }
    let (b0, b1) = (*b.first()?, *b.get(1)?);
    if b0 == 0xFF && b1 & 0xE0 == 0xE0 {
        let layer = (b1 >> 1) & 0x3;
        if b1 & 0xF0 == 0xF0 && layer == 0 {
            return Some(FileFormat::Aac);
        }
        if layer != 0 {
            return Some(FileFormat::Mp3);
        }
    }
    None
}

fn detect_by_ext(ext: &str) -> FileFormat {
    match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "wav" | "wave" | "bwf" | "rf64" => FileFormat::Wav,
        "aif" | "aiff" | "aifc" => FileFormat::Aiff,
        "flac" => FileFormat::Flac,
        "mp3" | "mp2" | "mp1" => FileFormat::Mp3,
        "ogg" | "oga" => FileFormat::Ogg,
        "aac" | "m4a" | "m4b" | "mp4" | "adts" => FileFormat::Aac,
        "caf" => FileFormat::Caf,
        _ => FileFormat::Other,
    }
}

pub fn detect_format(bytes: &[u8], ext_hint: Option<&str>) -> FileFormat {
    detect_by_magic(bytes, 0).unwrap_or_else(|| ext_hint.map(detect_by_ext).unwrap_or(FileFormat::Other))
}

pub fn probe(bytes: &[u8], ext_hint: Option<&str>) -> Result<AudioInfo> {
    let format = detect_format(bytes, ext_hint);
    match format {
        FileFormat::Wav if wav::is_wav(bytes) => match wav::parse(bytes)? {
            wav::WavParse::Native(l) => Ok(l.info),
            wav::WavParse::Foreign { bwf } => {
                let mut info = symph::probe(bytes, format)?;
                info.bwf = bwf;
                Ok(info)
            }
        },
        FileFormat::Aiff if aiff::is_aiff(bytes) => match aiff::parse(bytes)? {
            aiff::AiffParse::Native(l) => Ok(l.info),
            aiff::AiffParse::Foreign => symph::probe(bytes, format),
        },
        _ => symph::probe(bytes, format),
    }
}

pub fn decode(bytes: &[u8], ext_hint: Option<&str>) -> Result<(AudioInfo, AudioBuffer)> {
    let format = detect_format(bytes, ext_hint);
    match format {
        FileFormat::Wav if wav::is_wav(bytes) => match wav::parse(bytes)? {
            wav::WavParse::Native(l) => {
                let buf = wav::decode(bytes, &l)?;
                let mut info = l.info;
                info.frames = buf.frames() as u64;
                Ok((info, buf))
            }
            wav::WavParse::Foreign { bwf } => {
                let (mut info, buf) = symph::decode(bytes, format)?;
                info.bwf = bwf;
                Ok((info, buf))
            }
        },
        FileFormat::Aiff if aiff::is_aiff(bytes) => match aiff::parse(bytes)? {
            aiff::AiffParse::Native(l) => {
                let buf = aiff::decode(bytes, &l)?;
                let mut info = l.info;
                info.frames = buf.frames() as u64;
                Ok((info, buf))
            }
            aiff::AiffParse::Foreign => symph::decode(bytes, format),
        },
        _ => symph::decode(bytes, format),
    }
}

pub fn encode(buf: &AudioBuffer, opts: &EncodeOptions) -> Result<Vec<u8>> {
    match opts.format {
        FileFormat::Wav => wav::encode(buf, opts.bit_depth, opts.dither, opts.bwf.as_ref(), false, None),
        FileFormat::Aiff => aiff::encode(buf, opts.bit_depth, opts.dither),
        FileFormat::Flac => flac::encode(buf, opts.bit_depth, opts.dither),
        FileFormat::Mp3 => mp3::encode(buf),
        other => Err(AudioError::Unsupported(format!("encoding {other:?} is not supported"))),
    }
}

pub const ENCODE_EXTENSIONS: &str = "wav, aif, aiff, flac, mp3";

pub fn encode_format_for(name: &str) -> Option<FileFormat> {
    match name.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "wav" | "wave" | "bwf" => Some(FileFormat::Wav),
        "aif" | "aiff" => Some(FileFormat::Aiff),
        "flac" => Some(FileFormat::Flac),
        "mp3" => Some(FileFormat::Mp3),
        _ => None,
    }
}

pub fn encode_wav_rf64(buf: &AudioBuffer, opts: &EncodeOptions) -> Result<Vec<u8>> {
    wav::encode(buf, opts.bit_depth, opts.dither, opts.bwf.as_ref(), true, None)
}

pub fn encode_with_channel_mask(buf: &AudioBuffer, opts: &EncodeOptions, mask: u32) -> Result<Vec<u8>> {
    match opts.format {
        FileFormat::Wav => wav::encode(buf, opts.bit_depth, opts.dither, opts.bwf.as_ref(), false, Some(mask)),
        _ => encode(buf, opts),
    }
}

pub struct FileEncoder<W: Write + Seek>(Writer<W>);

enum Writer<W> {
    Wav(wav::Writer<W>),
    Aiff(aiff::Writer<W>),
    Flac(flac::Writer<W>),
    Mp3(W, AudioBuffer),
}

impl<W: Write + Seek> FileEncoder<W> {
    pub fn new(out: W, opts: &EncodeOptions, channels: usize, sample_rate: u32, mask: Option<u32>) -> Result<Self> {
        let (depth, dither) = (opts.bit_depth, opts.dither);
        Ok(FileEncoder(match opts.format {
            FileFormat::Wav => Writer::Wav(wav::Writer::new(out, channels, sample_rate, depth, dither, opts.bwf.as_ref(), mask, false)?),
            FileFormat::Aiff => Writer::Aiff(aiff::Writer::new(out, channels, sample_rate, depth, dither)?),
            FileFormat::Flac => Writer::Flac(flac::Writer::new(out, channels, sample_rate, depth, dither)?),
            FileFormat::Mp3 => Writer::Mp3(out, AudioBuffer { sample_rate, channels: vec![Vec::new(); channels.clamp(1, 2)] }),
            other => return Err(AudioError::Unsupported(format!("encoding {other:?} is not supported"))),
        }))
    }

    pub fn check_room(&self, frames: u64) -> Result<()> {
        match &self.0 {
            Writer::Wav(w) => w.check_room(frames),
            Writer::Aiff(w) => w.check_room(frames),
            Writer::Flac(w) => w.check_room(frames),
            Writer::Mp3(..) => Ok(frames),
        }
        .map(drop)
    }

    pub fn write(&mut self, block: &[Vec<f32>], frames: usize) -> Result<()> {
        match &mut self.0 {
            Writer::Wav(w) => w.write(block, frames),
            Writer::Aiff(w) => w.write(block, frames),
            Writer::Flac(w) => w.write(block, frames),
            Writer::Mp3(_, b) => {
                for (c, dst) in b.channels.iter_mut().enumerate() {
                    let src = block.get(c).map_or(&[][..], Vec::as_slice);
                    dst.extend((0..frames).map(|i| src.get(i).copied().unwrap_or(0.0)));
                }
                Ok(())
            }
        }
    }

    pub fn finish(self) -> Result<W> {
        match self.0 {
            Writer::Wav(w) => w.finish(),
            Writer::Aiff(w) => w.finish(),
            Writer::Flac(w) => w.finish(),
            Writer::Mp3(mut out, b) => {
                out.write_all(&mp3::encode(&b)?).map_err(|e| AudioError::Encode(e.to_string()))?;
                Ok(out)
            }
        }
    }
}

pub fn wav_channel_mask(bytes: &[u8]) -> Option<u32> {
    wav::read_channel_mask(bytes)
}

pub fn wav_float_header(channels: usize, sample_rate: u32, frames: u64) -> Result<Vec<u8>> {
    if channels == 0 || sample_rate == 0 {
        return Err(AudioError::Encode("a WAV needs channels and a sample rate".into()));
    }
    wav::header(channels, sample_rate, (32, true), None, None, frames, false)
}
