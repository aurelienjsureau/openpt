#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod automation;
pub mod b64;
mod clip;
mod ids;
mod learn;
mod markers;
mod mixer;
mod session;
mod source;
mod track;

pub use automation::{AutoParam, AutomationLane, AutomationMode, AutomationPoint, BYPASS_PARAM, SURROUND_AXES};
pub use clip::{Clip, ClipContent, Fade, FadeShape, Warp, WarpMarker};
pub use ids::{BusId, ClipId, GroupId, MarkerId, SourceId, TrackId};
pub use learn::{LearnSource, LearnTarget, MidiLearn};
pub use markers::{Group, MarkerKind, MemoryLocation};
pub use mixer::{
    AtmosMonitor, AtmosSettings, Bus, ChannelFormat, HeatSettings, INSERT_SLOTS, InputPath, Insert, MULTI_MONO_PREFIX, MULTI_OUT_PREFIX, Mixer, OutputPath, Route, SEND_SLOTS,
    SendSlot, Speaker, SurroundPan, default_inputs, fader_db_to_pos, fader_pos_to_db,
};
pub use session::{
    BitDepthSetting, EditMode, EditState, FEET_FRAMES_START, MAX_COMMENT_CHARS, MAX_NAME_CHARS, SESSION_EXTENSION, Session, SessionError, SessionLink, Tool,
    TrackView, ZoomState, ZoomToggle, truncate_chars,
};
pub use source::{Source, SourceAudio, SourcePool, VideoSource};
pub use track::{MidiFrom, Playlist, TRACK_COLORS, Track, TrackHeight, TrackKind};

pub use openpt_time as time;
