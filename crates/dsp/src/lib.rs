#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod biquad;
pub mod heat;
pub mod meter;
pub mod offline;
mod osc;
pub mod pan;
mod params;
pub mod pitch_detect;
pub mod plugins;
pub mod spectrum;
mod util;

pub use offline::FadeShape;
pub use osc::Waveform;
pub use plugins::eq::{eq1_response, eq7_response};

use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Category {
    Eq,
    Dynamics,
    PitchShift,
    Reverb,
    Delay,
    Modulation,
    Harmonic,
    Dither,
    Instrument,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub enum Unit {
    None,
    Db,
    Hz,
    Ms,
    Percent,
    Ratio,
    Semitones,
    Cents,
    Seconds,
    Toggle,
    Choice,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub enum Taper {
    Linear,
    Log,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ParamInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: Unit,
    pub taper: Taper,
    pub choices: &'static [&'static str],
}

impl ParamInfo {
    pub fn clamp(&self, value: f32) -> f32 {
        let v = if value.is_nan() { self.default } else { value };
        let v = v.clamp(self.min, self.max);
        match self.unit {
            Unit::Toggle | Unit::Choice => v.round().clamp(self.min, self.max),
            _ => v,
        }
    }

    pub fn to_normalized(&self, value: f32) -> f32 {
        let v = self.clamp(value);
        let span = self.max - self.min;
        if span <= 0.0 {
            return 0.0;
        }
        match self.taper {
            Taper::Log if self.min > 0.0 => ((v / self.min).ln() / (self.max / self.min).ln()).clamp(0.0, 1.0),
            _ => ((v - self.min) / span).clamp(0.0, 1.0),
        }
    }

    pub fn from_normalized(&self, t: f32) -> f32 {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        let v = match self.taper {
            Taper::Log if self.min > 0.0 => self.min * (self.max / self.min).powf(t),
            _ => self.min + (self.max - self.min) * t,
        };
        self.clamp(v)
    }

    pub fn format(&self, value: f32) -> String {
        let v = self.clamp(value);
        match self.unit {
            Unit::Choice => self.choices.get(v as usize).map(|s| (*s).to_string()).unwrap_or_else(|| format!("{v}")),
            Unit::Toggle => if v >= 0.5 { "On" } else { "Off" }.to_string(),
            Unit::Db => {
                if v <= -95.9 {
                    "-inf dB".to_string()
                } else {
                    format!("{v:.1} dB")
                }
            }
            Unit::Hz => {
                if v >= 1000.0 {
                    format!("{:.2} kHz", v / 1000.0)
                } else {
                    format!("{v:.1} Hz")
                }
            }
            Unit::Ms => format!("{v:.1} ms"),
            Unit::Percent => format!("{v:.0}%"),
            Unit::Ratio => format!("{v:.1}:1"),
            Unit::Semitones => format!("{v:.0} st"),
            Unit::Cents => format!("{v:.0} ct"),
            Unit::Seconds => format!("{v:.2} s"),
            Unit::None => format!("{v:.2}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PluginInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub short_name: &'static str,
    pub category: Category,
    pub params: &'static [ParamInfo],
    pub is_instrument: bool,
    pub audiosuite: bool,
}

impl PluginInfo {
    pub fn param(&self, id: &str) -> Option<&'static ParamInfo> {
        self.params.iter().find(|p| p.id == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Transport {
    pub playing: bool,
    pub recording: bool,
    pub tempo: f64,
    pub numerator: u32,
    pub denominator: u32,
    pub samples: i64,
    pub seconds: f64,
    pub quarters: f64,
    pub bar_start: f64,
    pub bar: i64,
}

pub trait Plugin: Send {
    fn info(&self) -> &'static PluginInfo;
    fn prepare(&mut self, sample_rate: f32, max_block: usize, channels: usize);
    fn reset(&mut self);
    fn set_param(&mut self, id: &str, value: f32) -> bool;
    fn param(&self, id: &str) -> Option<f32>;
    fn process(&mut self, io: &mut [Vec<f32>], frames: usize);
    fn latency(&self) -> usize {
        0
    }
    fn tail_samples(&self) -> usize {
        0
    }
    fn gain_reduction_db(&self) -> f32 {
        0.0
    }
    fn set_transport(&mut self, _transport: &Transport) {}
    fn note_on(&mut self, _offset: usize, _note: u8, _velocity: u8) {}
    fn note_on_chan(&mut self, offset: usize, _channel: u8, note: u8, velocity: u8) {
        self.note_on(offset, note, velocity);
    }
    fn note_off(&mut self, _offset: usize, _note: u8) {}
    fn note_off_chan(&mut self, offset: usize, _channel: u8, note: u8) {
        self.note_off(offset, note);
    }
    fn midi(&mut self, _offset: usize, _msg: [u8; 3]) {}
    fn all_notes_off(&mut self) {}
    fn set_sample(&mut self, _sample: Option<Arc<dyn SampleSource>>) -> bool {
        false
    }
    fn save_state(&mut self) -> Option<Vec<u8>> {
        self.state_reader()?.save_state()
    }
    fn load_state(&mut self, _data: &[u8]) -> bool {
        false
    }
    fn open_editor(&mut self) -> Result<(), String> {
        Err("no editor".into())
    }
    fn close_editor(&mut self) {}
    fn editor(&mut self) -> Option<Box<dyn PluginEditor>> {
        None
    }
    fn state_reader(&mut self) -> Option<Box<dyn PluginStateReader>> {
        None
    }
}

pub trait PluginEditor: Send {
    fn open(&mut self) -> Result<(), String>;
    fn close(&mut self);
    fn is_open(&self) -> bool;
    fn idle(&mut self) -> Vec<(String, f32)>;
    fn set_param(&mut self, _id: &str, _value: f32) {}
}

pub trait PluginStateReader: Send {
    fn save_state(&mut self) -> Option<Vec<u8>>;
}

impl<F: FnMut() -> Option<Vec<u8>> + Send> PluginStateReader for F {
    fn save_state(&mut self) -> Option<Vec<u8>> {
        self()
    }
}

pub trait SampleSource: Send + Sync {
    fn sample_rate(&self) -> f32;
    fn num_channels(&self) -> usize;
    fn channel(&self, ch: usize) -> Option<&[f32]>;
    fn frames(&self) -> usize {
        (0..self.num_channels()).filter_map(|c| self.channel(c)).map(<[f32]>::len).min().unwrap_or(0)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SampleBuffer {
    pub sample_rate: f32,
    pub channels: Vec<Vec<f32>>,
}

impl SampleSource for SampleBuffer {
    fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
    fn num_channels(&self) -> usize {
        self.channels.len()
    }
    fn channel(&self, ch: usize) -> Option<&[f32]> {
        self.channels.get(ch).map(Vec::as_slice)
    }
}

pub fn plugins() -> &'static [&'static PluginInfo] {
    plugins::REGISTRY
}

pub fn create(id: &str) -> Option<Box<dyn Plugin>> {
    let mut p = plugins::instantiate(id)?;
    p.prepare(48_000.0, 1024, 2);
    Some(p)
}

pub fn plugin_info(id: &str) -> Option<&'static PluginInfo> {
    plugins::REGISTRY.iter().copied().find(|i| i.id == id)
}

pub fn is_generator(id: &str) -> bool {
    id == "signal_generator"
}

pub const MIN_DB: f32 = -144.0;

pub fn db_to_gain(db: f32) -> f32 {
    if db.is_nan() || db <= MIN_DB { 0.0 } else { 10f32.powf(db.min(200.0) / 20.0) }
}

pub fn gain_to_db(g: f32) -> f32 {
    let a = g.abs();
    if a.is_nan() {
        return MIN_DB;
    }
    if a.is_infinite() {
        return f32::MAX;
    }
    (20.0 * a.log10()).max(MIN_DB)
}

pub fn midi_to_hz(note: f64) -> f64 {
    440.0 * 2f64.powf((note - 69.0) / 12.0)
}

#[cfg(test)]
mod trait_default_tests {
    use super::*;

    #[test]
    fn built_ins_have_no_state_or_editor() {
        for info in plugins() {
            let mut p = create(info.id).unwrap();
            assert!(p.save_state().is_none(), "{}", info.id);
            assert!(!p.load_state(b"anything"), "{}", info.id);
            assert!(p.open_editor().is_err(), "{}", info.id);
            p.close_editor();
            assert!(p.editor().is_none(), "{}", info.id);
        }
    }

    #[test]
    fn only_the_sampler_takes_samples() {
        for info in plugins() {
            let mut p = create(info.id).unwrap();
            let s: Arc<dyn SampleSource> = Arc::new(SampleBuffer { sample_rate: 48_000.0, channels: vec![vec![0.0; 16]] });
            assert_eq!(p.set_sample(Some(s)), info.id == "sampler", "{}", info.id);
            assert_eq!(p.set_sample(None), info.id == "sampler", "{}", info.id);
        }
    }

    #[test]
    fn sample_buffer_frames_is_the_shortest_channel() {
        let s = SampleBuffer { sample_rate: 44_100.0, channels: vec![vec![0.0; 10], vec![0.0; 7]] };
        assert_eq!(s.frames(), 7);
        assert_eq!(SampleBuffer::default().frames(), 0);
    }
}
