use crate::{ClipId, MAX_NAME_CHARS, SourceId, truncate_chars};
use openpt_midi::Sequence;
use openpt_time::{MAX_POSITION, Range, Samples};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum FadeShape {
    Linear,
    #[default]
    EqualPower,
    SCurve,
    Exponential,
    Logarithmic,
}

impl FadeShape {
    pub const ALL: [FadeShape; 5] = [FadeShape::Linear, FadeShape::EqualPower, FadeShape::SCurve, FadeShape::Exponential, FadeShape::Logarithmic];

    pub fn gain(self, t: f32) -> f32 {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        match self {
            FadeShape::Linear => t,
            FadeShape::EqualPower => (t * std::f32::consts::FRAC_PI_2).sin(),
            FadeShape::SCurve => 0.5 - 0.5 * (t * std::f32::consts::PI).cos(),
            FadeShape::Exponential => t * t * t,
            FadeShape::Logarithmic => 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FadeShape::Linear => "Linear",
            FadeShape::EqualPower => "Equal Power",
            FadeShape::SCurve => "S-Curve",
            FadeShape::Exponential => "Exponential",
            FadeShape::Logarithmic => "Logarithmic",
        }
    }

    pub fn from_id(s: &str) -> Option<FadeShape> {
        let s = s.to_ascii_lowercase().replace(['-', '_', ' '], "");
        FadeShape::ALL.into_iter().find(|f| f.label().to_ascii_lowercase().replace(['-', ' '], "") == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Fade {
    pub len: Samples,
    pub shape: FadeShape,
    #[serde(default)]
    pub curvature: f32,
}

impl Fade {
    pub fn gain(self, t: f32) -> f32 {
        let base = self.shape.gain(t);
        let curvature = if self.curvature.is_finite() { self.curvature.clamp(-1.0, 1.0) } else { 0.0 };
        if curvature == 0.0 || base <= 0.0 || base >= 1.0 {
            return base;
        }
        let weighted = base * (curvature * 4.0).exp2();
        weighted / (weighted + 1.0 - base)
    }

    pub fn curvature_for_midpoint_gain(self, gain: f32) -> f32 {
        let gain = if gain.is_finite() { gain.clamp(0.001, 0.999) } else { self.gain(0.5) };
        let base = self.shape.gain(0.5);
        (((gain / (1.0 - gain)).log2() - (base / (1.0 - base)).log2()) / 4.0).clamp(-1.0, 1.0)
    }

    pub fn sanitize(&mut self) {
        self.curvature = if self.curvature.is_finite() { self.curvature.clamp(-1.0, 1.0) } else { 0.0 };
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ClipContent {
    Audio { source: SourceId, offset: Samples },
    Midi { sequence: Sequence },
    Video { source: SourceId, offset: Samples },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub name: String,
    pub start: Samples,
    pub length: Samples,
    pub content: ClipContent,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub gain_env: Vec<(Samples, f32)>,
    #[serde(default)]
    pub fade_in: Fade,
    #[serde(default)]
    pub fade_out: Fade,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub edit_locked: bool,
    #[serde(default)]
    pub time_locked: bool,
    #[serde(default)]
    pub rating: u8,
    #[serde(default)]
    pub sync_point: Samples,
    #[serde(default)]
    pub color: Option<[u8; 3]>,
    #[serde(default = "one")]
    pub stretch: f64,
    #[serde(default)]
    pub group: Option<u64>,
    #[serde(default)]
    pub loop_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warp: Option<Warp>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WarpMarker {
    pub src: Samples,
    pub at: Samples,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Warp {
    pub source: SourceId,
    pub offset: Samples,
    pub length: Samples,
    #[serde(default)]
    pub markers: Vec<WarpMarker>,
    #[serde(default)]
    pub rendered: Option<SourceId>,
}

impl Warp {
    pub fn anchors(&self, clip_len: Samples) -> Vec<WarpMarker> {
        let mut v = vec![WarpMarker { src: 0, at: 0 }];
        v.extend(self.markers.iter().copied().filter(|m| m.src > 0 && m.src < self.length));
        v.push(WarpMarker { src: self.length.max(1), at: clip_len.max(1) });
        v
    }

    pub fn to_clip(&self, clip_len: Samples, src: Samples) -> Samples {
        Self::map(&self.anchors(clip_len), src, |m| (m.src, m.at))
    }

    pub fn to_src(&self, clip_len: Samples, at: Samples) -> Samples {
        Self::map(&self.anchors(clip_len), at, |m| (m.at, m.src))
    }

    fn map(anchors: &[WarpMarker], x: Samples, key: impl Fn(&WarpMarker) -> (Samples, Samples)) -> Samples {
        let pts: Vec<(Samples, Samples)> = anchors.iter().map(key).collect();
        let seg = pts.windows(2).find(|w| x <= w[1].0).or_else(|| pts.windows(2).last());
        match seg {
            Some([a, b]) if b.0 > a.0 => a.1 + ((x - a.0) as f64 * (b.1 - a.1) as f64 / (b.0 - a.0) as f64).round() as Samples,
            Some([a, _]) => a.1,
            _ => x,
        }
    }
}

fn one() -> f64 {
    1.0
}

impl Clip {
    pub fn audio(id: ClipId, name: impl Into<String>, source: SourceId, offset: Samples, start: Samples, length: Samples) -> Clip {
        Clip {
            id,
            name: name.into(),
            start,
            length: length.max(1),
            content: ClipContent::Audio { source, offset },
            gain_db: 0.0,
            gain_env: Vec::new(),
            fade_in: Fade::default(),
            fade_out: Fade::default(),
            muted: false,
            edit_locked: false,
            time_locked: false,
            rating: 0,
            sync_point: 0,
            color: None,
            stretch: 1.0,
            group: None,
            loop_id: None,
            warp: None,
        }
    }

    pub fn midi(id: ClipId, name: impl Into<String>, start: Samples, length: Samples, sequence: Sequence) -> Clip {
        let mut c = Clip::audio(id, name, SourceId(0), 0, start, length);
        c.content = ClipContent::Midi { sequence };
        c
    }

    pub fn video(id: ClipId, name: impl Into<String>, source: SourceId, offset: Samples, start: Samples, length: Samples) -> Clip {
        let mut c = Clip::audio(id, name, SourceId(0), 0, start, length);
        c.content = ClipContent::Video { source, offset };
        c
    }

    pub fn end(&self) -> Samples {
        self.start.saturating_add(self.length)
    }

    pub fn range(&self) -> Range {
        Range { start: self.start, end: self.end() }
    }

    pub fn is_audio(&self) -> bool {
        matches!(self.content, ClipContent::Audio { .. })
    }

    pub fn is_video(&self) -> bool {
        matches!(self.content, ClipContent::Video { .. })
    }

    pub fn source(&self) -> Option<SourceId> {
        match self.content {
            ClipContent::Audio { source, .. } => Some(source),
            ClipContent::Midi { .. } | ClipContent::Video { .. } => None,
        }
    }

    pub fn video_source(&self) -> Option<SourceId> {
        match self.content {
            ClipContent::Video { source, .. } => Some(source),
            ClipContent::Audio { .. } | ClipContent::Midi { .. } => None,
        }
    }

    pub fn source_offset(&self) -> Samples {
        match self.content {
            ClipContent::Audio { offset, .. } | ClipContent::Video { offset, .. } => offset,
            ClipContent::Midi { .. } => 0,
        }
    }

    pub fn gain_db_at(&self, rel: Samples) -> f32 {
        self.gain_db + if self.gain_env.is_empty() { 0.0 } else { env_value(&self.gain_env, rel) }
    }

    pub fn gain_at(&self, rel: Samples) -> f32 {
        self.gain_at_with_fades(rel, self.fade_in, self.fade_out)
    }

    pub fn gain_at_with_fades(&self, rel: Samples, fade_in: Fade, fade_out: Fade) -> f32 {
        let mut db = self.gain_db;
        if !self.gain_env.is_empty() {
            db += env_value(&self.gain_env, rel);
        }
        let mut g = db_to_gain(db);
        if fade_in.len > 0 && rel < fade_in.len {
            g *= fade_in.gain(rel as f32 / fade_in.len as f32);
        }
        let from_end = self.length.saturating_sub(rel);
        if fade_out.len > 0 && from_end <= fade_out.len {
            g *= fade_out.gain(from_end as f32 / fade_out.len as f32);
        }
        g
    }

    pub fn trim_start_to(&mut self, new_start: Samples) {
        let new_start = new_start.min(self.end() - 1);
        let delta = new_start - self.start;
        if let ClipContent::Audio { offset, .. } | ClipContent::Video { offset, .. } = &mut self.content {
            *offset = offset.saturating_add(delta);
        }
        self.length -= delta;
        self.start = new_start;
        if self.sync_point != 0 {
            self.sync_point = self.sync_point.saturating_sub(delta);
        }
        self.drop_stray_sync();
        self.clamp_fades();
        if self.gain_env.iter().any(|&(t, _)| t < delta) && self.gain_env.iter().all(|&(t, _)| t != delta) {
            self.gain_env.insert(0, (delta, env_value(&self.gain_env, delta)));
        }
        self.gain_env.retain(|&(t, _)| t >= delta);
        self.gain_env.iter_mut().for_each(|(t, _)| *t = t.saturating_sub(delta));
    }

    pub fn trim_end_to(&mut self, new_end: Samples) {
        let new_end = new_end.max(self.start + 1);
        self.length = new_end - self.start;
        self.drop_stray_sync();
        self.clamp_fades();
    }

    pub fn clamp_fades(&mut self) {
        self.fade_in.len = self.fade_in.len.clamp(0, self.length);
        self.fade_out.len = self.fade_out.len.clamp(0, self.length);
    }

    fn drop_stray_sync(&mut self) {
        if !(0..self.length).contains(&self.sync_point) {
            self.sync_point = 0;
        }
    }

    pub fn sanitize(&mut self) {
        bound(&mut self.start);
        self.length = self.length.clamp(1, MAX_POSITION);
        bound(&mut self.sync_point);
        self.gain_env.iter_mut().for_each(|(at, _)| bound(at));
        match &mut self.content {
            ClipContent::Audio { offset, .. } | ClipContent::Video { offset, .. } => bound(offset),
            ClipContent::Midi { sequence } => bound_sequence(sequence),
        }
        self.clamp_fades();
        self.fade_in.sanitize();
        self.fade_out.sanitize();
        truncate_chars(&mut self.name, MAX_NAME_CHARS);
        if !self.gain_db.is_finite() {
            self.gain_db = 0.0;
        }
        if !self.stretch.is_finite() || self.stretch <= 0.0 {
            self.stretch = 1.0;
        }
    }
}

pub(crate) fn bound(at: &mut Samples) {
    *at = (*at).clamp(-MAX_POSITION, MAX_POSITION);
}

pub(crate) fn bound_sequence(s: &mut Sequence) {
    for n in &mut s.notes {
        bound(&mut n.start);
        n.length = n.length.clamp(1, MAX_POSITION);
    }
    s.ctrls.iter_mut().for_each(|c| bound(&mut c.tick));
}

fn env_value(env: &[(Samples, f32)], at: Samples) -> f32 {
    let mut prev: Option<(Samples, f32)> = None;
    for &(t, v) in env {
        if t >= at {
            return match prev {
                Some((pt, pv)) if t > pt => {
                    let elapsed = i128::from(at).saturating_sub(i128::from(pt));
                    let span = i128::from(t).saturating_sub(i128::from(pt));
                    pv + (v - pv) * (elapsed as f32 / span as f32)
                }
                _ => v,
            };
        }
        prev = Some((t, v));
    }
    prev.map_or(0.0, |(_, v)| v)
}

pub(crate) fn db_to_gain(db: f32) -> f32 {
    if db.is_nan() || db <= -144.0 { 0.0 } else { 10f32.powf(db.min(48.0) / 20.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warp_maps_both_ways_through_its_markers() {
        let w = Warp { source: SourceId(1), offset: 0, length: 1000, markers: vec![WarpMarker { src: 500, at: 300 }], rendered: None };
        assert_eq!(w.to_clip(1000, 250), 150);
        assert_eq!(w.to_clip(1000, 500), 300);
        assert_eq!(w.to_clip(1000, 750), 650);
        assert_eq!(w.to_src(1000, 650), 750);
        assert_eq!(w.to_src(1000, 0), 0);
        assert_eq!(w.to_clip(2000, 1000), 2000);
    }

    #[test]
    fn fade_shapes_hit_endpoints() {
        for s in FadeShape::ALL {
            assert!(s.gain(0.0).abs() < 1e-6, "{s:?}");
            assert!((s.gain(1.0) - 1.0).abs() < 1e-6, "{s:?}");
            assert_eq!(s.gain(f32::NAN), 0.0);
            assert_eq!(FadeShape::from_id(s.label()), Some(s));
        }
    }

    #[test]
    fn fade_curvature_is_bounded_monotonic_and_inverts_at_the_midpoint() {
        for shape in FadeShape::ALL {
            let original = Fade { len: 1000, shape, curvature: 0.0 };
            for i in 0..=128 {
                let t = i as f32 / 128.0;
                assert_eq!(original.gain(t), shape.gain(t), "legacy curves must be unchanged");
            }
            for curvature in [-1.0, -0.5, 0.0, 0.5, 1.0] {
                let fade = Fade { curvature, ..original };
                assert_eq!(fade.gain(0.0), 0.0);
                assert_eq!(fade.gain(1.0), 1.0);
                let mut previous = 0.0;
                for i in 0..=128 {
                    let gain = fade.gain(i as f32 / 128.0);
                    assert!(gain.is_finite() && (0.0..=1.0).contains(&gain));
                    assert!(gain >= previous, "{shape:?}, {curvature}");
                    previous = gain;
                }
                assert!((fade.curvature_for_midpoint_gain(fade.gain(0.5)) - curvature).abs() < 1e-5);
            }
            assert!((Fade { curvature: 0.5, ..original }).gain(0.5) > original.gain(0.5));
            assert!((Fade { curvature: -0.5, ..original }).gain(0.5) < original.gain(0.5));
        }
    }

    #[test]
    fn fade_curvature_loads_legacy_data_and_sanitizes_hostile_values() {
        let fade: Fade = serde_json::from_str(r#"{"len":100,"shape":"SCurve"}"#).unwrap();
        assert_eq!(fade.curvature, 0.0);
        for curvature in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -10.0, 10.0] {
            let mut fade = Fade { curvature, ..fade };
            assert!(fade.gain(0.5).is_finite());
            fade.sanitize();
            assert!(fade.curvature.is_finite() && (-1.0..=1.0).contains(&fade.curvature));
            let roundtrip: Fade = serde_json::from_str(&serde_json::to_string(&fade).unwrap()).unwrap();
            assert_eq!(roundtrip, fade);
        }
    }

    #[test]
    fn temporary_fades_keep_clip_gain_and_envelope_without_mutating_the_clip() {
        let mut clip = Clip::audio(ClipId(1), "preview", SourceId(1), 0, 0, 1000);
        clip.gain_db = -6.0;
        clip.gain_env = vec![(0, 0.0), (1000, 6.0)];
        clip.fade_in = Fade { len: 100, shape: FadeShape::SCurve, curvature: 0.25 };
        clip.fade_out = Fade { len: 100, shape: FadeShape::Logarithmic, curvature: -0.1 };
        let before = clip.clone();
        let fade_in = Fade { len: 1000, shape: FadeShape::Linear, curvature: 0.5 };
        let fade_out = Fade { len: 1000, shape: FadeShape::Linear, curvature: -0.5 };
        assert!((clip.gain_at_with_fades(500, fade_in, fade_out) - db_to_gain(-3.0) * 0.16).abs() < 1e-6);
        let mut committed = clip.clone();
        committed.fade_in = fade_in;
        committed.fade_out = fade_out;
        for rel in [0, 50, 500, 999, 1000] {
            assert_eq!(clip.gain_at_with_fades(rel, fade_in, fade_out), committed.gain_at(rel));
        }
        for rel in [i64::MIN, i64::MAX] {
            assert!(clip.gain_at(rel).is_finite());
            assert!(clip.gain_at_with_fades(rel, fade_in, fade_out).is_finite());
        }
        assert_eq!(clip, before);
        clip.gain_env = vec![(i64::MIN, 0.0), (i64::MAX, 6.0)];
        assert!((clip.gain_at(500) - db_to_gain(-3.0)).abs() < 1e-6);
        assert!((clip.gain_at_with_fades(500, fade_in, fade_out) - db_to_gain(-3.0) * 0.16).abs() < 1e-6);
    }

    #[test]
    fn gain_with_fades() {
        let mut c = Clip::audio(ClipId(1), "a", SourceId(1), 0, 100, 1000);
        c.fade_in = Fade { len: 100, shape: FadeShape::Linear, curvature: 0.0 };
        c.fade_out = Fade { len: 100, shape: FadeShape::Linear, curvature: 0.0 };
        assert!(c.gain_at(0).abs() < 1e-6);
        assert!((c.gain_at(50) - 0.5).abs() < 1e-6);
        assert!((c.gain_at(500) - 1.0).abs() < 1e-6);
        assert!((c.gain_at(950) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn gain_envelope_interpolates() {
        let mut c = Clip::audio(ClipId(1), "a", SourceId(1), 0, 0, 1000);
        c.gain_env = vec![(0, 0.0), (1000, -20.0)];
        assert!((c.gain_at(500) - db_to_gain(-10.0)).abs() < 1e-4);
    }

    #[test]
    fn trim_start_keeps_gain_envelope_at_cut() {
        for (env, cut, expected) in [
            (vec![(0, 0.0), (1000, -20.0)], 500, vec![(0, -10.0), (500, -20.0)]),
            (vec![(0, 0.0), (100, -20.0)], 500, vec![(0, -20.0)]),
            (vec![(0, 0.0), (500, -10.0), (1000, -20.0)], 500, vec![(0, -10.0), (500, -20.0)]),
            (vec![], 500, vec![]),
            (vec![(100, -3.0)], 0, vec![(100, -3.0)]),
            (vec![(100, -3.0)], -100, vec![(200, -3.0)]),
        ] {
            let mut clip = Clip::audio(ClipId(1), "a", SourceId(1), 100, 0, 1000);
            clip.gain_env = env;
            let gain_at_cut = clip.gain_at(cut);
            clip.trim_start_to(cut);
            assert_eq!(clip.gain_env, expected);
            if cut > 0 {
                assert!((clip.gain_at(0) - gain_at_cut).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn trims_keep_material_anchored() {
        let mut c = Clip::audio(ClipId(1), "a", SourceId(1), 1000, 100, 1000);
        c.trim_start_to(300);
        assert_eq!((c.start, c.length, c.source_offset()), (300, 800, 1200));
        c.trim_end_to(500);
        assert_eq!(c.length, 200);
        c.trim_end_to(-50);
        assert_eq!(c.length, 1);
        let mut c = Clip::audio(ClipId(2), "b", SourceId(1), 0, 0, 1000);
        c.fade_out = Fade { len: 800, shape: FadeShape::Linear, curvature: 0.0 };
        c.trim_start_to(900);
        assert!(c.fade_out.len <= c.length);
    }
}
