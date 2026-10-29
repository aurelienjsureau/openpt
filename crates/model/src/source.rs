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
enum Samples {
    Owned(Vec<Vec<f32>>),
    #[cfg(not(target_arch = "wasm32"))]
    Mapped(openpt_mapped::MappedSamples),
}

#[derive(Debug)]
pub struct SourceAudio {
    samples: Samples,
    sample_rate: u32,
    frames: usize,
    pub peaks: Vec<Peaks>,
}

impl SourceAudio {
    pub fn new(buffer: AudioBuffer) -> Self {
        let peaks = buffer.channels.iter().map(|c| Peaks::build(c)).collect();
        let frames = buffer.frames();
        SourceAudio { samples: Samples::Owned(buffer.channels), sample_rate: buffer.sample_rate, frames, peaks }
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn mapped(samples: openpt_mapped::MappedSamples) -> Self {
        let peaks = (0..samples.channels()).map(|c| Peaks::build(samples.channel(c).unwrap_or(&[]))).collect();
        SourceAudio { sample_rate: samples.sample_rate(), frames: samples.frames(), samples: Samples::Mapped(samples), peaks }
    }
    pub fn is_mapped(&self) -> bool {
        !matches!(self.samples, Samples::Owned(_))
    }
    pub fn frames(&self) -> usize {
        self.frames
    }
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn num_channels(&self) -> usize {
        match &self.samples {
            Samples::Owned(c) => c.len(),
            #[cfg(not(target_arch = "wasm32"))]
            Samples::Mapped(m) => m.channels(),
        }
    }
    pub fn channel(&self, ch: usize) -> Option<&[f32]> {
        match &self.samples {
            Samples::Owned(c) => c.get(ch).map(Vec::as_slice),
            #[cfg(not(target_arch = "wasm32"))]
            Samples::Mapped(m) => m.channel(ch),
        }
    }
    pub fn channels(&self) -> Vec<&[f32]> {
        (0..self.num_channels()).filter_map(|c| self.channel(c)).collect()
    }
    pub fn sample(&self, ch: usize, frame: usize) -> f32 {
        self.channel(ch).and_then(|c| c.get(frame)).copied().unwrap_or(0.0)
    }
    pub fn to_buffer(&self) -> AudioBuffer {
        AudioBuffer { sample_rate: self.sample_rate, channels: self.channels().into_iter().map(<[f32]>::to_vec).collect() }
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
