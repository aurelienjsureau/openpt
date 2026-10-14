#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod automation;
pub mod b64;
mod clip;
mod ids;
mod markers;
mod mixer;
mod session;
mod source;
mod track;

pub use automation::{AutoParam, AutomationLane, AutomationMode, AutomationPoint};
pub use clip::{Clip, ClipContent, Fade, FadeShape, Warp, WarpMarker};
pub use ids::{BusId, ClipId, GroupId, MarkerId, SourceId, TrackId};
pub use markers::{Group, MarkerKind, MemoryLocation};
pub use mixer::{
    Bus, ChannelFormat, INSERT_SLOTS, InputPath, Insert, MULTI_MONO_PREFIX, Mixer, OutputPath, Route, default_inputs, SEND_SLOTS, SendSlot, Speaker, SurroundPan,
    fader_db_to_pos, fader_pos_to_db,
};
pub use session::{
    BitDepthSetting, EditMode, EditState, FEET_FRAMES_START, MAX_COMMENT_CHARS, MAX_NAME_CHARS, SESSION_EXTENSION, Session, SessionError, Tool,
    TrackView, ZoomState, ZoomToggle, truncate_chars,
};
pub use source::{Source, SourceAudio, SourcePool, VideoSource};
pub use track::{Playlist, TRACK_COLORS, Track, TrackHeight, TrackKind};

pub use openpt_time as time;
