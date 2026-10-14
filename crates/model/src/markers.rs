use crate::{GroupId, MarkerId, TrackId};
use openpt_time::Samples;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum MarkerKind {
    #[default]
    Marker,
    Selection,
    None,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemoryLocation {
    pub id: MarkerId,
    pub number: u32,
    pub name: String,
    pub kind: MarkerKind,
    pub start: Samples,
    pub end: Samples,
    #[serde(default)]
    pub comments: String,
    #[serde(default)]
    pub color: Option<[u8; 3]>,
    #[serde(default = "one")]
    pub ruler: u8,
    #[serde(default)]
    pub track: Option<TrackId>,
}

fn one() -> u8 {
    1
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub letter: char,
    pub members: Vec<TrackId>,
    pub edit: bool,
    pub mix: bool,
    pub active: bool,
    pub color: [u8; 3],
    #[serde(default)]
    pub attributes: Vec<String>,
}
