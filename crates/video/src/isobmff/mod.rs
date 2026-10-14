mod bytes;
mod codec;
mod demux;
mod error;
mod mux;
mod source;

pub use bytes::FourCc;
pub use codec::{
    AacConfig, AudioParams, Av1Config, AvcConfig, BitRate, CleanAperture, CodecConfig, ColorInfo, FieldInfo, FlacConfig, HevcConfig, HevcNalArray,
    MasteringDisplay, OpusConfig, PcmConfig, SampleEntry, TimecodeConfig, VideoParams, VpcConfig, parse_asc,
};
pub use demux::{Edit, Metadata, Mp4File, Sample, SampleGroup, Track, TrackKind, open};
pub use error::{Error, Result};
pub use mux::{Brand, FragmentedWriter, Mp4Writer, TrackConfig, WriteSample, WriterOptions};
pub use source::ByteSource;
