#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod live;
mod names;
pub mod ops;
mod rng;
mod sequence;
mod smf;

pub use live::{LiveMidiEvent, MidiTake, RecordedMidiEvent, parse_midi_message};
pub use names::{gm_drum_name, gm_program_name, note_name};
pub use rng::XorShift;
pub use sequence::{CtrlEvent, CtrlKind, Note, Sequence};
pub use smf::{MAX_EVENTS, MAX_TRACKS, Smf, SmfTrack, read_smf, write_smf};

pub const PPQ: u32 = 960;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MidiError {
    #[error("not a Standard MIDI File (missing MThd header)")]
    NotSmf,
    #[error("unexpected end of data at byte {0}")]
    Truncated(usize),
    #[error("variable-length quantity longer than 4 bytes at byte {0}")]
    VlqTooLong(usize),
    #[error("chunk length {len} at byte {offset} exceeds the remaining data")]
    BadChunkLength { offset: usize, len: u64 },
    #[error("invalid MIDI data at byte {offset}: {reason}")]
    Invalid { offset: usize, reason: &'static str },
    #[error("unsupported MIDI file: {0}")]
    Unsupported(&'static str),
    #[error("too many events (limit {0})")]
    TooManyEvents(usize),
    #[error("too many tracks (limit {0})")]
    TooManyTracks(usize),
    #[error("cannot write MIDI file: {0}")]
    Write(String),
}
