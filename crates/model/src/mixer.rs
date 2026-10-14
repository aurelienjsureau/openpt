use crate::{AutomationMode, BusId, SourceId};
use std::collections::BTreeMap;

pub const INSERT_SLOTS: usize = 10;
pub const SEND_SLOTS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ChannelFormat {
    #[default]
    Mono,
    Stereo,
    Lcr,
    Quad,
    Lcrs,
    Surround50,
    Surround51,
    Surround60,
    Surround61,
    Sdds70,
    Sdds71,
    Surround70,
    Surround71,
    Atmos702,
    Atmos712,
    Atmos502,
    Atmos512,
    Atmos504,
    Atmos514,
    Atmos704,
    Atmos714,
    Atmos706,
    Atmos716,
    Atmos904,
    Atmos914,
    Atmos906,
    Atmos916,
    Ambisonics(u8),
}

impl ChannelFormat {
    pub const ALL: [ChannelFormat; 34] = [
        ChannelFormat::Mono,
        ChannelFormat::Stereo,
        ChannelFormat::Lcr,
        ChannelFormat::Quad,
        ChannelFormat::Lcrs,
        ChannelFormat::Surround50,
        ChannelFormat::Surround51,
        ChannelFormat::Surround60,
        ChannelFormat::Surround61,
        ChannelFormat::Sdds70,
        ChannelFormat::Sdds71,
        ChannelFormat::Surround70,
        ChannelFormat::Surround71,
        ChannelFormat::Atmos702,
        ChannelFormat::Atmos712,
        ChannelFormat::Ambisonics(1),
        ChannelFormat::Ambisonics(2),
        ChannelFormat::Ambisonics(3),
        ChannelFormat::Ambisonics(4),
        ChannelFormat::Ambisonics(5),
        ChannelFormat::Ambisonics(6),
        ChannelFormat::Ambisonics(7),
        ChannelFormat::Atmos502,
        ChannelFormat::Atmos512,
        ChannelFormat::Atmos504,
        ChannelFormat::Atmos514,
        ChannelFormat::Atmos704,
        ChannelFormat::Atmos714,
        ChannelFormat::Atmos706,
        ChannelFormat::Atmos716,
        ChannelFormat::Atmos904,
        ChannelFormat::Atmos914,
        ChannelFormat::Atmos906,
        ChannelFormat::Atmos916,
    ];
    pub fn channels(self) -> usize {
        match self {
            ChannelFormat::Mono => 1,
            ChannelFormat::Stereo => 2,
            ChannelFormat::Lcr => 3,
            ChannelFormat::Quad | ChannelFormat::Lcrs => 4,
            ChannelFormat::Surround50 => 5,
            ChannelFormat::Surround51 | ChannelFormat::Surround60 => 6,
            ChannelFormat::Surround61 | ChannelFormat::Sdds70 | ChannelFormat::Surround70 | ChannelFormat::Atmos502 => 7,
            ChannelFormat::Sdds71 | ChannelFormat::Surround71 | ChannelFormat::Atmos512 => 8,
            ChannelFormat::Atmos702 | ChannelFormat::Atmos504 => 9,
            ChannelFormat::Atmos712 | ChannelFormat::Atmos514 => 10,
            ChannelFormat::Atmos704 => 11,
            ChannelFormat::Atmos714 => 12,
            ChannelFormat::Atmos706 | ChannelFormat::Atmos904 => 13,
            ChannelFormat::Atmos716 | ChannelFormat::Atmos914 => 14,
            ChannelFormat::Atmos906 => 15,
            ChannelFormat::Atmos916 => 16,
            ChannelFormat::Ambisonics(n) => {
                let n = usize::from(n.clamp(1, 7)) + 1;
                n * n
            }
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ChannelFormat::Mono => "Mono",
            ChannelFormat::Stereo => "Stereo",
            ChannelFormat::Lcr => "LCR",
            ChannelFormat::Quad => "Quad",
            ChannelFormat::Lcrs => "LCRS",
            ChannelFormat::Surround50 => "5.0",
            ChannelFormat::Surround51 => "5.1",
            ChannelFormat::Surround60 => "6.0",
            ChannelFormat::Surround61 => "6.1",
            ChannelFormat::Sdds70 => "7.0 SDDS",
            ChannelFormat::Sdds71 => "7.1 SDDS",
            ChannelFormat::Surround70 => "7.0",
            ChannelFormat::Surround71 => "7.1",
            ChannelFormat::Atmos702 => "7.0.2",
            ChannelFormat::Atmos712 => "7.1.2",
            ChannelFormat::Atmos502 => "5.0.2",
            ChannelFormat::Atmos512 => "5.1.2",
            ChannelFormat::Atmos504 => "5.0.4",
            ChannelFormat::Atmos514 => "5.1.4",
            ChannelFormat::Atmos704 => "7.0.4",
            ChannelFormat::Atmos714 => "7.1.4",
            ChannelFormat::Atmos706 => "7.0.6",
            ChannelFormat::Atmos716 => "7.1.6",
            ChannelFormat::Atmos904 => "9.0.4",
            ChannelFormat::Atmos914 => "9.1.4",
            ChannelFormat::Atmos906 => "9.0.6",
            ChannelFormat::Atmos916 => "9.1.6",
            ChannelFormat::Ambisonics(1) => "1st Order Ambisonics",
            ChannelFormat::Ambisonics(2) => "2nd Order Ambisonics",
            ChannelFormat::Ambisonics(3) => "3rd Order Ambisonics",
            ChannelFormat::Ambisonics(4) => "4th Order Ambisonics",
            ChannelFormat::Ambisonics(5) => "5th Order Ambisonics",
            ChannelFormat::Ambisonics(6) => "6th Order Ambisonics",
            ChannelFormat::Ambisonics(_) => "7th Order Ambisonics",
        }
    }
    pub fn from_id(s: &str) -> Option<ChannelFormat> {
        ChannelFormat::ALL.into_iter().find(|f| f.label().eq_ignore_ascii_case(s.trim()))
    }
    pub fn speakers(self) -> &'static [Speaker] {
        use Speaker::*;
        match self {
            ChannelFormat::Mono => &[C],
            ChannelFormat::Stereo => &[L, R],
            ChannelFormat::Lcr => &[L, R, C],
            ChannelFormat::Quad => &[L, R, Ls, Rs],
            ChannelFormat::Lcrs => &[L, R, C, Cs],
            ChannelFormat::Surround50 => &[L, R, C, Ls, Rs],
            ChannelFormat::Surround51 => &[L, R, C, Lfe, Ls, Rs],
            ChannelFormat::Surround60 => &[L, R, C, Ls, Rs, Cs],
            ChannelFormat::Surround61 => &[L, R, C, Lfe, Ls, Rs, Cs],
            ChannelFormat::Sdds70 => &[L, R, C, Ls, Rs, Lc, Rc],
            ChannelFormat::Sdds71 => &[L, R, C, Lfe, Ls, Rs, Lc, Rc],
            ChannelFormat::Surround70 => &[L, R, C, Lrs, Rrs, Lss, Rss],
            ChannelFormat::Surround71 => &[L, R, C, Lfe, Lrs, Rrs, Lss, Rss],
            ChannelFormat::Atmos702 => &[L, R, C, Lrs, Rrs, Lss, Rss, Ltm, Rtm],
            ChannelFormat::Atmos712 => &[L, R, C, Lfe, Lrs, Rrs, Lss, Rss, Ltm, Rtm],
            ChannelFormat::Atmos502 => &[L, R, C, Ls, Rs, Ltm, Rtm],
            ChannelFormat::Atmos512 => &[L, R, C, Lfe, Ls, Rs, Ltm, Rtm],
            ChannelFormat::Atmos504 => &[L, R, C, Ls, Rs, Ltf, Rtf, Ltr, Rtr],
            ChannelFormat::Atmos514 => &[L, R, C, Lfe, Ls, Rs, Ltf, Rtf, Ltr, Rtr],
            ChannelFormat::Atmos704 => &[L, R, C, Lrs, Rrs, Lss, Rss, Ltf, Rtf, Ltr, Rtr],
            ChannelFormat::Atmos714 => &[L, R, C, Lfe, Lrs, Rrs, Lss, Rss, Ltf, Rtf, Ltr, Rtr],
            ChannelFormat::Atmos706 => &[L, R, C, Lrs, Rrs, Lss, Rss, Ltf, Rtf, Ltr, Rtr, Ltm, Rtm],
            ChannelFormat::Atmos716 => &[L, R, C, Lfe, Lrs, Rrs, Lss, Rss, Ltf, Rtf, Ltr, Rtr, Ltm, Rtm],
            ChannelFormat::Atmos904 => &[L, R, C, Lrs, Rrs, Lw, Rw, Lss, Rss, Ltf, Rtf, Ltr, Rtr],
            ChannelFormat::Atmos914 => &[L, R, C, Lfe, Lrs, Rrs, Lw, Rw, Lss, Rss, Ltf, Rtf, Ltr, Rtr],
            ChannelFormat::Atmos906 => &[L, R, C, Lrs, Rrs, Lw, Rw, Lss, Rss, Ltf, Rtf, Ltr, Rtr, Ltm, Rtm],
            ChannelFormat::Atmos916 => &[L, R, C, Lfe, Lrs, Rrs, Lw, Rw, Lss, Rss, Ltf, Rtf, Ltr, Rtr, Ltm, Rtm],
            ChannelFormat::Ambisonics(_) => &[],
        }
    }
    pub fn is_ambisonic(self) -> bool {
        matches!(self, ChannelFormat::Ambisonics(_))
    }
    pub fn has_lfe(self) -> bool {
        self.speakers().contains(&Speaker::Lfe)
    }
    pub fn has_height(self) -> bool {
        self.speakers().iter().any(|s| s.is_height())
    }
    pub fn channel_label(self, ch: usize) -> String {
        match self.speakers().get(ch) {
            Some(s) => s.label().to_string(),
            None if self.is_ambisonic() => ["W", "Y", "Z", "X"].get(ch).map_or_else(|| format!("{}", ch + 1), |s| s.to_string()),
            None => format!("{}", ch + 1),
        }
    }
    pub fn channel_mask(self) -> u32 {
        let sp = self.speakers();
        let has_tf = sp.contains(&Speaker::Ltf);
        sp.iter().fold(0u32, |m, s| match s {
            Speaker::Ltm if !has_tf => m | 0x1000,
            Speaker::Rtm if !has_tf => m | 0x4000,
            s => m | s.wave_bit(),
        })
    }
    pub fn for_channels(n: usize) -> ChannelFormat {
        [
            ChannelFormat::Mono,
            ChannelFormat::Stereo,
            ChannelFormat::Lcr,
            ChannelFormat::Quad,
            ChannelFormat::Surround50,
            ChannelFormat::Surround51,
            ChannelFormat::Surround70,
            ChannelFormat::Surround71,
        ]
        .into_iter()
        .find(|f| f.channels() == n)
        .unwrap_or(if n <= 1 { ChannelFormat::Mono } else { ChannelFormat::Stereo })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Speaker {
    L,
    R,
    C,
    Lfe,
    Ls,
    Rs,
    Lc,
    Rc,
    Cs,
    Lss,
    Rss,
    Lrs,
    Rrs,
    Lw,
    Rw,
    Ltf,
    Rtf,
    Ltm,
    Rtm,
    Ltr,
    Rtr,
}

impl Speaker {
    pub fn azimuth(self) -> f32 {
        match self {
            Speaker::L => -30.0,
            Speaker::R => 30.0,
            Speaker::C | Speaker::Lfe => 0.0,
            Speaker::Ls => -110.0,
            Speaker::Rs => 110.0,
            Speaker::Lc => -15.0,
            Speaker::Rc => 15.0,
            Speaker::Cs => 180.0,
            Speaker::Lss => -90.0,
            Speaker::Rss => 90.0,
            Speaker::Lrs => -150.0,
            Speaker::Rrs => 150.0,
            Speaker::Lw => -60.0,
            Speaker::Rw => 60.0,
            Speaker::Ltf => -45.0,
            Speaker::Rtf => 45.0,
            Speaker::Ltm => -90.0,
            Speaker::Rtm => 90.0,
            Speaker::Ltr => -135.0,
            Speaker::Rtr => 135.0,
        }
    }
    pub fn elevation(self) -> f32 {
        if self.is_height() { 45.0 } else { 0.0 }
    }
    pub fn is_height(self) -> bool {
        matches!(self, Speaker::Ltf | Speaker::Rtf | Speaker::Ltm | Speaker::Rtm | Speaker::Ltr | Speaker::Rtr)
    }
    pub fn is_lfe(self) -> bool {
        self == Speaker::Lfe
    }
    pub fn label(self) -> &'static str {
        match self {
            Speaker::L => "L",
            Speaker::R => "R",
            Speaker::C => "C",
            Speaker::Lfe => "LFE",
            Speaker::Ls => "Ls",
            Speaker::Rs => "Rs",
            Speaker::Lc => "Lc",
            Speaker::Rc => "Rc",
            Speaker::Cs => "Cs",
            Speaker::Lss => "Lss",
            Speaker::Rss => "Rss",
            Speaker::Lrs => "Lrs",
            Speaker::Rrs => "Rrs",
            Speaker::Lw => "Lw",
            Speaker::Rw => "Rw",
            Speaker::Ltf => "Ltf",
            Speaker::Rtf => "Rtf",
            Speaker::Ltm => "Ltm",
            Speaker::Rtm => "Rtm",
            Speaker::Ltr => "Ltr",
            Speaker::Rtr => "Rtr",
        }
    }
    pub fn wave_bit(self) -> u32 {
        match self {
            Speaker::L => 0x1,
            Speaker::R => 0x2,
            Speaker::C => 0x4,
            Speaker::Lfe => 0x8,
            Speaker::Ls | Speaker::Lrs => 0x10,
            Speaker::Rs | Speaker::Rrs => 0x20,
            Speaker::Lc | Speaker::Lw => 0x40,
            Speaker::Rc | Speaker::Rw => 0x80,
            Speaker::Cs => 0x100,
            Speaker::Lss => 0x200,
            Speaker::Rss => 0x400,
            Speaker::Ltf => 0x1000,
            Speaker::Rtf => 0x4000,
            Speaker::Ltr => 0x8000,
            Speaker::Rtr => 0x2_0000,
            Speaker::Ltm | Speaker::Rtm => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SurroundPan {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub divergence: f32,
    pub center: f32,
    pub lfe_db: f32,
}

impl Default for SurroundPan {
    fn default() -> Self {
        SurroundPan { x: 0.0, y: 1.0, z: 0.0, divergence: 0.0, center: 100.0, lfe_db: -144.0 }
    }
}

impl SurroundPan {
    pub fn from_stereo(pan: f32) -> Self {
        SurroundPan { x: finite_or(pan, 0.0).clamp(-1.0, 1.0), ..SurroundPan::default() }
    }
    pub fn sanitize(&mut self) {
        let d = SurroundPan::default();
        self.x = finite_or(self.x, d.x).clamp(-1.0, 1.0);
        self.y = finite_or(self.y, d.y).clamp(-1.0, 1.0);
        self.z = finite_or(self.z, d.z).clamp(0.0, 1.0);
        self.divergence = finite_or(self.divergence, d.divergence).clamp(0.0, 1.0);
        self.center = finite_or(self.center, d.center).clamp(0.0, 100.0);
        self.lfe_db = finite_or(self.lfe_db, d.lfe_db).clamp(-144.0, 12.0);
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bus {
    pub id: BusId,
    pub name: String,
    pub format: ChannelFormat,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OutputPath {
    pub name: String,
    pub first_channel: u16,
    pub format: ChannelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct HeatSettings {
    pub active: bool,
    pub bypass: bool,
    pub drive: f32,
    pub tone: f32,
}

impl Default for HeatSettings {
    fn default() -> Self {
        HeatSettings { active: false, bypass: false, drive: 0.5, tone: 0.0 }
    }
}

pub type InputPath = OutputPath;

pub fn default_inputs() -> Vec<InputPath> {
    let mono = (0..8u16).map(|c| InputPath { name: format!("In {}", c + 1), first_channel: c, format: ChannelFormat::Mono });
    let pairs = (0..4u16).map(|p| InputPath { name: format!("In {}-{}", 2 * p + 1, 2 * p + 2), first_channel: 2 * p, format: ChannelFormat::Stereo });
    mono.chain(pairs).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum Route {
    #[default]
    None,
    Main,
    Bus(BusId),
    Hardware(String),
}

pub const MULTI_MONO_PREFIX: &str = "mm:";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Insert {
    pub plugin: String,
    #[serde(default, deserialize_with = "finite_params")]
    pub params: BTreeMap<String, f32>,
    #[serde(default)]
    pub bypass: bool,
    #[serde(default = "yes")]
    pub active: bool,
    #[serde(default)]
    pub preset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<SourceId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multi_mono: bool,
}

fn yes() -> bool {
    true
}

impl Insert {
    pub fn new(plugin: impl Into<String>) -> Self {
        Insert {
            plugin: plugin.into(),
            params: BTreeMap::new(),
            bypass: false,
            active: true,
            preset: String::from("<factory default>"),
            state: None,
            sample: None,
            multi_mono: false,
        }
    }

    pub fn instance_id(&self) -> String {
        if self.multi_mono { format!("{MULTI_MONO_PREFIX}{}", self.plugin) } else { self.plugin.clone() }
    }

    pub fn is_instance(&self, id: &str) -> bool {
        if self.multi_mono { id.strip_prefix(MULTI_MONO_PREFIX) == Some(self.plugin.as_str()) } else { id == self.plugin }
    }

    pub fn state_bytes(&self) -> Option<Vec<u8>> {
        self.state.as_deref().and_then(crate::b64::decode)
    }

    pub fn set_state_bytes(&mut self, data: &[u8]) {
        self.state = (!data.is_empty()).then(|| crate::b64::encode(data));
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SendSlot {
    pub target: Route,
    pub level_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub pre_fader: bool,
    #[serde(default)]
    pub follow_main_pan: bool,
    #[serde(default)]
    pub surround: Option<SurroundPan>,
}

impl SendSlot {
    pub fn new(target: Route) -> Self {
        SendSlot { target, level_db: -144.0, pan: 0.0, mute: false, pre_fader: false, follow_main_pan: false, surround: None }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Mixer {
    pub volume_db: f32,
    pub pan: Vec<f32>,
    pub mute: bool,
    pub solo: bool,
    #[serde(default)]
    pub solo_safe: bool,
    #[serde(default)]
    pub record_safe: bool,
    #[serde(default)]
    pub record_arm: bool,
    #[serde(default)]
    pub input_monitor: bool,
    #[serde(default)]
    pub phase_invert: bool,
    pub input: Route,
    pub output: Route,
    pub inserts: Vec<Option<Insert>>,
    pub sends: Vec<Option<SendSlot>>,
    pub automation_mode: AutomationMode,
    #[serde(default)]
    pub trim_db: f32,
    #[serde(default)]
    pub heat_bypass: bool,
    #[serde(default)]
    pub heat_pre: bool,
    #[serde(default)]
    pub vca: Option<u64>,
    #[serde(default)]
    pub surround: Option<SurroundPan>,
}

impl Mixer {
    pub fn new(format: ChannelFormat) -> Self {
        let pan = match format {
            ChannelFormat::Mono => vec![0.0],
            ChannelFormat::Stereo => vec![-1.0, 1.0],
            _ => Vec::new(),
        };
        Mixer {
            volume_db: 0.0,
            pan,
            mute: false,
            solo: false,
            solo_safe: false,
            record_safe: false,
            record_arm: false,
            input_monitor: false,
            phase_invert: false,
            input: Route::None,
            output: Route::Main,
            inserts: vec![None; INSERT_SLOTS],
            sends: vec![None; SEND_SLOTS],
            automation_mode: AutomationMode::Read,
            trim_db: 0.0,
            heat_bypass: false,
            heat_pre: false,
            vca: None,
            surround: None,
        }
    }

    pub fn sanitize(&mut self) {
        self.volume_db = finite_or(self.volume_db, 0.0).clamp(-144.0, 12.0);
        for p in &mut self.pan {
            *p = finite_or(*p, 0.0).clamp(-1.0, 1.0);
        }
        self.inserts.resize(INSERT_SLOTS, None);
        self.sends.resize(SEND_SLOTS, None);
        for s in self.sends.iter_mut().flatten() {
            s.level_db = finite_or(s.level_db, -144.0).clamp(-144.0, 12.0);
            s.pan = finite_or(s.pan, 0.0).clamp(-1.0, 1.0);
            if let Some(sp) = &mut s.surround {
                sp.sanitize();
            }
        }
        if let Some(sp) = &mut self.surround {
            sp.sanitize();
        }
        self.trim_db = finite_or(self.trim_db, 0.0).clamp(-144.0, 24.0);
    }
}

impl crate::Session {
    pub fn main_format(&self) -> ChannelFormat {
        match self.outputs.first().map(|o| o.format) {
            Some(f) if (2..=16).contains(&f.channels()) => f,
            _ => ChannelFormat::Stereo,
        }
    }
}

pub(crate) fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

fn finite_params<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, f32>, D::Error> {
    let params: BTreeMap<String, Option<f64>> = serde::Deserialize::deserialize(d)?;
    Ok(params.into_iter().filter_map(|(k, v)| Some((k, v.map(|v| v as f32).filter(|v| v.is_finite())?))).collect())
}

pub fn fader_pos_to_db(pos: f32) -> f32 {
    let p = finite_or(pos, 0.0).clamp(0.0, 1.0);
    if p <= 0.0 {
        return -144.0;
    }
    if p >= 0.25 { -24.0 + (p - 0.25) / 0.75 * 36.0 } else { -24.0 - (1.0 - p / 0.25).powf(1.5) * 120.0 }
}

pub fn fader_db_to_pos(db: f32) -> f32 {
    let db = finite_or(db, -144.0);
    if db <= -144.0 {
        return 0.0;
    }
    if db >= -24.0 {
        (0.25 + (db.min(12.0) + 24.0) / 36.0 * 0.75).min(1.0)
    } else {
        let x = ((-24.0 - db) / 120.0).clamp(0.0, 1.0).powf(1.0 / 1.5);
        ((1.0 - x) * 0.25).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_mono_flag_round_trips_and_defaults_off() {
        let mut i = Insert::new("vst3:00");
        assert_eq!(i.instance_id(), "vst3:00");
        assert!(!serde_json::to_string(&i).unwrap().contains("multi_mono"));
        i.multi_mono = true;
        assert_eq!(i.instance_id(), "mm:vst3:00");
        assert!(i.is_instance("mm:vst3:00") && !i.is_instance("vst3:00"));
        let back: Insert = serde_json::from_str(&serde_json::to_string(&i).unwrap()).unwrap();
        assert!(back.multi_mono);
        let old: Insert = serde_json::from_str(r#"{"plugin":"eq7"}"#).unwrap();
        assert!(!old.multi_mono && old.is_instance("eq7"));
    }

    #[test]
    fn insert_state_round_trips_through_json() {
        let mut i = Insert::new("vst3:00");
        assert_eq!(i.state_bytes(), None);
        let blob: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        i.set_state_bytes(&blob);
        let json = serde_json::to_string(&i).unwrap();
        let back: Insert = serde_json::from_str(&json).unwrap();
        assert_eq!(back.state_bytes().unwrap(), blob);
        i.set_state_bytes(&[]);
        assert_eq!(i.state, None);
        let old: Insert = serde_json::from_str(r#"{"plugin":"eq_7band"}"#).unwrap();
        assert_eq!(old.state, None);
        assert!(!serde_json::to_string(&old).unwrap().contains("state"));
        let bad: Insert = serde_json::from_str(r#"{"plugin":"x","state":"!!!"}"#).unwrap();
        assert_eq!(bad.state_bytes(), None);
    }

    #[test]
    fn params_saved_as_null_or_out_of_range_are_dropped() {
        let mut i = Insert::new("eq_7band");
        i.params.extend([("gain".to_string(), f32::NAN), ("q".to_string(), 0.7)]);
        let back: Insert = serde_json::from_str(&serde_json::to_string(&i).unwrap()).unwrap();
        assert_eq!(back.params, BTreeMap::from([("q".to_string(), 0.7)]));
        let big: Insert = serde_json::from_str(r#"{"plugin":"x","params":{"gain":1e39,"q":0.5}}"#).unwrap();
        assert_eq!(big.params, BTreeMap::from([("q".to_string(), 0.5)]));
    }

    #[test]
    fn fader_taper_round_trips() {
        for db in [-100.0f32, -60.0, -24.0, -10.0, 0.0, 6.0, 12.0] {
            let p = fader_db_to_pos(db);
            assert!((fader_pos_to_db(p) - db).abs() < 0.05, "{db} → {p}");
        }
        assert_eq!(fader_pos_to_db(0.0), -144.0);
        assert_eq!(fader_db_to_pos(f32::NAN), 0.0);
    }

    #[test]
    fn mixer_defaults_and_sanitize() {
        let mut m = Mixer::new(ChannelFormat::Stereo);
        assert_eq!(m.pan, vec![-1.0, 1.0]);
        m.volume_db = f32::INFINITY;
        m.inserts.truncate(2);
        m.sanitize();
        assert_eq!(m.volume_db, 0.0);
        assert_eq!(m.inserts.len(), INSERT_SLOTS);
    }

    #[test]
    fn speaker_layouts_match_channel_counts_and_masks() {
        for f in ChannelFormat::ALL {
            if !f.is_ambisonic() {
                assert_eq!(f.speakers().len(), f.channels(), "{f:?}");
                let bits: Vec<u32> = f.speakers().iter().map(|s| s.wave_bit()).filter(|b| *b != 0).collect();
                assert!(bits.windows(2).all(|w| w[0] < w[1]), "{f:?} not in WAV order");
            }
        }
        assert_eq!(ChannelFormat::Surround51.channel_mask(), 0x3F);
        assert_eq!(ChannelFormat::Surround71.channel_mask(), 0x63F);
        assert_eq!(ChannelFormat::Atmos714.channel_mask(), 0x2_D63F);
        assert_eq!(ChannelFormat::Ambisonics(1).channel_mask(), 0);
    }

    #[test]
    fn surround_pan_defaults_and_old_sessions() {
        let m = Mixer::new(ChannelFormat::Mono);
        assert_eq!(m.surround, None);
        let mut v = serde_json::to_value(&m).unwrap();
        v.as_object_mut().unwrap().remove("surround");
        let back: Mixer = serde_json::from_value(v).unwrap();
        assert_eq!(back.surround, None);
        let mut sp: SurroundPan = serde_json::from_str(r#"{"x": 5.0}"#).unwrap();
        sp.sanitize();
        assert_eq!(sp.x, 1.0);
        assert_eq!(sp.center, 100.0);
    }

    #[test]
    fn formats() {
        assert_eq!(ChannelFormat::for_channels(6), ChannelFormat::Surround51);
        assert_eq!(ChannelFormat::from_id("stereo"), Some(ChannelFormat::Stereo));
    }
}
