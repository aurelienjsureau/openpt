use crate::SourceId;
use openpt_audio_io::{AudioBuffer, FileFormat, peaks::Peaks};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Source {
    pub id: SourceId,
    pub name: String,
    pub path: String,
    pub channels: u16,
    pub frames: u64,
    pub sample_rate: u32,
    pub format: FileFormat,
    #[serde(default)]
    pub time_reference: u64,
    #[serde(default)]
    pub unsaved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<openpt_audio_io::FieldInfo>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VideoSource {
    pub id: SourceId,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub frame_rate: f64,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub codec: String,
}

impl VideoSource {
    pub fn aspect(&self) -> f64 {
        if self.width == 0 || self.height == 0 { 16.0 / 9.0 } else { f64::from(self.width) / f64::from(self.height) }
    }
}

#[derive(Debug)]
pub struct SourceAudio {
    pub buffer: AudioBuffer,
    pub peaks: Vec<Peaks>,
}

impl SourceAudio {
    pub fn new(buffer: AudioBuffer) -> Self {
        let peaks = buffer.channels.iter().map(|c| Peaks::build(c)).collect();
        SourceAudio { buffer, peaks }
    }
    pub fn frames(&self) -> usize {
        self.buffer.frames()
    }
    pub fn channel(&self, ch: usize) -> Option<&[f32]> {
        self.buffer.channels.get(ch).map(Vec::as_slice)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SourcePool {
    map: BTreeMap<SourceId, Arc<SourceAudio>>,
}

impl SourcePool {
    pub fn insert(&mut self, id: SourceId, audio: Arc<SourceAudio>) {
        self.map.insert(id, audio);
    }
    pub fn get(&self, id: SourceId) -> Option<&Arc<SourceAudio>> {
        self.map.get(&id)
    }
    pub fn remove(&mut self, id: SourceId) -> Option<Arc<SourceAudio>> {
        self.map.remove(&id)
    }
    pub fn contains(&self, id: SourceId) -> bool {
        self.map.contains_key(&id)
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    pub fn ids(&self) -> impl Iterator<Item = SourceId> + '_ {
        self.map.keys().copied()
    }
}

impl PartialEq for SourcePool {
    fn eq(&self, other: &Self) -> bool {
        self.map.len() == other.map.len() && self.map.iter().zip(other.map.iter()).all(|((a, x), (b, y))| a == b && Arc::ptr_eq(x, y))
    }
}
