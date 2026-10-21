use crate::params::{Params, choice, lin, param_plumbing, toggle};
use crate::scope::{Column, Scope};
use crate::{Category, ParamInfo, Plugin, PluginInfo, Unit};
use std::sync::Arc;

pub const OUT_MODES: &[&str] = &["Audio + MIDI", "Audio", "MIDI"];
pub const FOCUS: &[&str] = &["Full Range", "Kick", "Snare", "Toms"];

const P_SENS: usize = 0;
const P_RETRIG: usize = 1;
const P_FLOOR: usize = 2;
const P_TRIG: usize = 3;
const P_OUT: usize = 4;
const P_NOTE: usize = 5;
const P_DYNVEL: usize = 6;
const P_FOCUS: usize = 7;
const P_LISTEN: usize = 8;

static PARAMS: [ParamInfo; 9] = [
    lin("sensitivity", "Sensitivity", 0.0, 100.0, 55.0, Unit::Percent),
    crate::params::log("retrigger", "Retrigger", 5.0, 300.0, 40.0, Unit::Ms),
    lin("floor", "Floor", -72.0, -12.0, -45.0, Unit::Db),
    lin("triglevel", "Trig Level", -24.0, 0.0, -3.0, Unit::Db),
    choice("outmode", "Output", OUT_MODES, 0),
    lin("note", "MIDI Note", 0.0, 127.0, 36.0, Unit::None),
    toggle("dynvel", "Dynamic Velocity", true),
    choice("focus", "Detection Focus", FOCUS, 0),
    toggle("listen", "Listen", false),
];

pub static OPEN_DRT_INFO: PluginInfo = PluginInfo {
    id: "open_drt",
    name: "OpenDRT",
    short_name: "OpenDRT",
    category: Category::Other,
    params: &PARAMS,
    is_instrument: false,
    audiosuite: false,
};

const VEL_REF_DB: f32 = -54.0;

struct Detector {
    sr: f32,
    att_fast: f32,
    rel_fast: f32,
    att_slow: f32,
    rel_slow: f32,
    att_med: f32,
    rel_med: f32,
    env_fast: f32,
    env_slow: f32,
    env_med: f32,
    ratio_thresh: f32,
    rearm: f32,
    esc_ratio: f32,
    floor: f32,
    guard_n: usize,
    guard: usize,
    armed: bool,
    peak_window: usize,
    peak_hunt: usize,
    peak_level: f32,
    since_peak: usize,
    last_peak_age: usize,
    last_hit_peak: f32,
    local_min: f32,
}

impl Detector {
    fn new(sr: f32) -> Detector {
        let one = |s: f32| (-1.0 / (sr * s.max(1.0e-5))).exp();
        Detector {
            sr,
            att_fast: one(0.0005),
            rel_fast: one(0.030),
            att_slow: one(0.030),
            rel_slow: one(0.150),
            att_med: one(0.002),
            rel_med: one(0.012),
            env_fast: 0.0,
            env_slow: 0.0,
            env_med: 0.0,
            ratio_thresh: 2.0,
            rearm: 1.5,
            esc_ratio: 1.6,
            floor: 0.01,
            guard_n: 1764,
            guard: 0,
            armed: true,
            peak_window: (sr * 0.008).round() as usize,
            peak_hunt: 0,
            peak_level: 0.0,
            since_peak: 0,
            last_peak_age: 0,
            last_hit_peak: 0.0,
            local_min: 1.0e9,
        }
    }

    fn set(&mut self, sens: f32, floor_db: f32, retrig_ms: f32) {
        let map = |v: f32, b0: f32, b1: f32| b0 + (b1 - b0) * v.clamp(0.0, 100.0) / 100.0;
        self.ratio_thresh = map(sens, 3.5, 1.05);
        self.rearm = 1.0 + (self.ratio_thresh - 1.0) * 0.5;
        self.esc_ratio = map(sens, 2.4, 1.6);
        self.floor = crate::db_to_gain(floor_db);
        self.guard_n = (retrig_ms.max(1.0) * 0.001 * self.sr).round() as usize;
    }

    fn threshold(&self) -> f32 {
        self.env_slow.max(self.floor * 0.5) * self.ratio_thresh
    }

    #[inline]
    fn follow(env: f32, x: f32, a: f32, r: f32) -> f32 {
        let c = if x > env { a } else { r };
        x + c * (env - x)
    }

    fn process(&mut self, x: f32) -> f32 {
        let rect = x.abs();
        self.env_fast = Self::follow(self.env_fast, rect, self.att_fast, self.rel_fast);
        self.env_slow = Self::follow(self.env_slow, rect, self.att_slow, self.rel_slow);
        self.env_med = Self::follow(self.env_med, rect, self.att_med, self.rel_med);
        self.local_min = self.local_min.min(self.env_med);
        self.guard = self.guard.saturating_sub(1);
        let reference = self.env_slow.max(self.floor * 0.5);
        let ratio = self.env_fast / (reference + 1.0e-9);
        if self.peak_hunt > 0 {
            if rect > self.peak_level {
                self.peak_level = rect;
                self.since_peak = 0;
            } else {
                self.since_peak += 1;
            }
            self.peak_hunt -= 1;
            if self.peak_hunt == 0 {
                self.last_peak_age = self.since_peak;
                self.last_hit_peak = self.peak_level;
                self.local_min = self.env_med;
                return self.peak_level;
            }
        } else if self.guard == 0 && self.env_fast > self.floor {
            let normal = self.armed && ratio >= self.ratio_thresh;
            let escalate = self.env_fast >= self.last_hit_peak * self.esc_ratio && ratio >= 1.10;
            let second = self.last_hit_peak > 0.0
                && self.local_min <= 0.5 * self.last_hit_peak
                && self.env_med >= self.local_min * 1.8
                && self.env_med >= 0.5 * self.last_hit_peak
                && self.env_fast >= 1.2 * self.env_med;
            if normal || escalate || second {
                self.armed = false;
                self.guard = self.guard_n;
                self.peak_hunt = self.peak_window.max(1);
                self.peak_level = rect;
                self.since_peak = 0;
                self.local_min = self.env_med;
            }
        }
        if !self.armed && ratio <= self.rearm {
            self.armed = true;
        }
        0.0
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Pending {
    remain: usize,
    level: f32,
    raw: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct Bq {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Bq {
    fn make(high: bool, sr: f32, fc: f32) -> Bq {
        let w0 = std::f64::consts::TAU * fc as f64 / sr as f64;
        let (c, s) = (w0.cos(), w0.sin());
        let alpha = s / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let (b0, b1) = if high { ((1.0 + c) * 0.5, -(1.0 + c)) } else { ((1.0 - c) * 0.5, 1.0 - c) };
        Bq {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b0 / a0) as f32,
            a1: (-2.0 * c / a0) as f32,
            a2: ((1.0 - alpha) / a0) as f32,
            z1: 0.0,
            z2: 0.0,
        }
    }
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

const HIST: usize = 8192;

pub struct OpenDrt {
    p: Params,
    sr: f32,
    det: Detector,
    lookahead: usize,
    edge_release: f32,
    edge_env: Vec<f32>,
    rect: Vec<f32>,
    rect_idx: usize,
    pending: [Pending; 16],
    focus: usize,
    hp: Bq,
    lp: Bq,
    spike_len: usize,
    spike_decay: f32,
    spike_left: usize,
    spike: f32,
    gate_len: usize,
    gate_left: usize,
    note_on: Option<u8>,
    midi: Vec<(usize, [u8; 3])>,
    col_hop: usize,
    col_n: usize,
    col_peak: f32,
    col_thresh: f32,
    col_hit: f32,
    held: Vec<Column>,
    keep_cols: usize,
    scope: Arc<Scope>,
}

impl Default for OpenDrt {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenDrt {
    pub fn new() -> OpenDrt {
        OpenDrt {
            p: Params::new(&PARAMS),
            sr: 48_000.0,
            det: Detector::new(48_000.0),
            lookahead: 0,
            edge_release: 0.0,
            edge_env: Vec::new(),
            rect: vec![0.0; HIST],
            rect_idx: 0,
            pending: [Pending::default(); 16],
            focus: usize::MAX,
            hp: Bq::default(),
            lp: Bq::default(),
            spike_len: 0,
            spike_decay: 0.0,
            spike_left: 0,
            spike: 0.0,
            gate_len: 0,
            gate_left: 0,
            note_on: None,
            midi: Vec::with_capacity(256),
            col_hop: 288,
            col_n: 0,
            col_peak: 0.0,
            col_thresh: 0.0,
            col_hit: 0.0,
            held: Vec::with_capacity(64),
            keep_cols: 1,
            scope: Scope::new(),
        }
    }

    fn update(&mut self) {}

    fn set_focus(&mut self, focus: usize) {
        self.focus = focus;
        let (hp, lp): (f32, f32) = match focus {
            1 => (30.0, 140.0),
            2 => (130.0, 600.0),
            3 => (50.0, 320.0),
            _ => (20.0, 20_000.0),
        };
        self.hp = Bq::make(true, self.sr, hp);
        self.lp = Bq::make(false, self.sr, lp.min(self.sr * 0.45));
    }

    fn push_midi(&mut self, offset: usize, msg: [u8; 3]) {
        if self.midi.len() < self.midi.capacity() {
            self.midi.push((offset, msg));
        }
    }

    fn fire(&mut self, i: usize, level: f32, raw: f32, want_audio: bool, want_midi: bool) {
        let note = self.p.v(P_NOTE).round().clamp(0.0, 127.0) as u8;
        let vel = if self.p.on(P_DYNVEL) {
            let db = crate::gain_to_db(raw.max(1.0e-6));
            let n = ((db - VEL_REF_DB) / -VEL_REF_DB).clamp(0.0, 1.0);
            (1.0 + (n * 126.0).round()).clamp(1.0, 127.0) as u8
        } else {
            110
        };
        if want_midi {
            if let Some(n) = self.note_on {
                self.push_midi(i, [0x80, n, 0]);
            }
            self.push_midi(i, [0x90, note, vel]);
            self.note_on = Some(note);
            self.gate_left = self.gate_len;
        }
        if want_audio {
            let trig = crate::db_to_gain(self.p.v(P_TRIG));
            let floor = crate::db_to_gain(self.p.v(P_FLOOR));
            self.spike_left = self.spike_len;
            self.spike = (trig * (level / (floor * 4.0).max(1.0e-3)).clamp(0.25, 1.0)).min(trig);
        }
    }
}

impl Plugin for OpenDrt {
    param_plumbing!(OPEN_DRT_INFO);

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, _channels: usize) {
        self.sr = if sample_rate.is_finite() && sample_rate > 0.0 { sample_rate.clamp(8_000.0, 384_000.0) } else { 48_000.0 };
        self.det = Detector::new(self.sr);
        let backtrack = (self.sr * 0.020).round() as usize;
        self.lookahead = (self.det.peak_window + backtrack).min(HIST - 1);
        self.edge_release = (-1.0 / (self.sr * 0.0015)).exp();
        self.edge_env = vec![0.0; self.lookahead.max(1)];
        self.rect.iter_mut().for_each(|v| *v = 0.0);
        self.rect_idx = 0;
        self.pending = [Pending::default(); 16];
        self.focus = usize::MAX;
        self.col_hop = ((self.sr * 0.006).round() as usize).max(1);
        self.col_n = 0;
        self.col_peak = 0.0;
        self.col_thresh = 0.0;
        self.col_hit = 0.0;
        self.held.clear();
        self.keep_cols = self.lookahead.div_ceil(self.col_hop).max(1);
        self.spike_len = (self.sr * 0.004).round() as usize;
        self.spike_decay = 0.001f32.powf(1.0 / self.spike_len.max(1) as f32);
        self.gate_len = (self.sr * 0.030).round() as usize;
        self.spike_left = 0;
        self.note_on = None;
    }

    fn reset(&mut self) {
        self.det = Detector::new(self.sr);
        self.pending = [Pending::default(); 16];
        self.spike_left = 0;
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        if io.is_empty() {
            return;
        }
        let n = frames.min(io.iter().map(Vec::len).min().unwrap_or(0));
        let mode = self.p.choice(P_OUT);
        let (want_audio, want_midi) = (mode != 2, mode != 1);
        let focus = self.p.choice(P_FOCUS);
        if focus != self.focus {
            self.set_focus(focus);
        }
        self.det.set(self.p.v(P_SENS), self.p.v(P_FLOOR), self.p.v(P_RETRIG));
        let listen = self.p.on(P_LISTEN);
        if !want_midi && let Some(note) = self.note_on.take() {
            self.push_midi(0, [0x80, note, 0]);
        }
        let mask = HIST - 1;
        let la = self.lookahead;
        for i in 0..n {
            for k in 0..self.pending.len() {
                let ph = self.pending[k];
                if ph.remain > 0 {
                    self.pending[k].remain -= 1;
                    if ph.remain == 1 {
                        self.fire(i, ph.level, ph.raw, want_audio, want_midi);
                    }
                }
            }
            let mono = if io.len() > 1 { 0.5 * (io[0][i] + io[1][i]) } else { io[0][i] };
            self.rect[self.rect_idx & mask] = mono.abs();
            let sc = if self.focus != 0 { self.lp.run(self.hp.run(mono)) } else { mono };
            let hit = self.det.process(sc);
            if hit > 0.0 {
                let (mut e, mut pk, mut peak_n) = (0.0f32, 0.0f32, la.saturating_sub(1));
                for k in 0..la {
                    let r = self.rect[(self.rect_idx + HIST - (la - 1) + k) & mask];
                    e = if r > e { r } else { r + self.edge_release * (e - r) };
                    self.edge_env[k] = e;
                    if e >= pk {
                        pk = e;
                        peak_n = k;
                    }
                }
                let mut onset_age = self.det.peak_window;
                if pk > 0.0 {
                    let foot = 0.15 * pk;
                    let mut nc = peak_n;
                    while nc > 0 && self.edge_env[nc - 1] > foot {
                        nc -= 1;
                    }
                    onset_age = (la - 1) - nc;
                }
                let mut raw = 0.0f32;
                for k in 0..self.det.peak_window {
                    raw = raw.max(self.rect[(self.rect_idx + HIST - k) & mask]);
                }
                let delay = la.saturating_sub(onset_age).max(1);
                if let Some(ph) = self.pending.iter_mut().find(|p| p.remain == 0) {
                    *ph = Pending { remain: delay, level: hit, raw };
                }
                let age = self.det.last_peak_age;
                if age <= self.col_n || self.held.is_empty() {
                    self.col_hit = self.col_hit.max(hit);
                } else {
                    let back = 1 + (age - self.col_n - 1) / self.col_hop;
                    let idx = self.held.len().saturating_sub(back);
                    if let Some(c) = self.held.get_mut(idx) {
                        c.hit = c.hit.max(crate::gain_to_db(hit.max(1.0e-7)));
                    }
                }
            }
            self.rect_idx = self.rect_idx.wrapping_add(1);
            self.col_peak = self.col_peak.max(sc.abs());
            self.col_thresh = self.col_thresh.max(self.det.threshold());
            self.col_n += 1;
            if self.col_n >= self.col_hop {
                let col = Column {
                    input: crate::gain_to_db(self.col_peak.max(1.0e-7)),
                    output: f32::NEG_INFINITY,
                    gr: 0.0,
                    aux: crate::gain_to_db(self.col_thresh.max(1.0e-7)),
                    hit: if self.col_hit > 0.0 { crate::gain_to_db(self.col_hit.max(1.0e-7)) } else { f32::NEG_INFINITY },
                };
                if self.held.len() < self.held.capacity() {
                    self.held.push(col);
                }
                if self.held.len() > self.keep_cols {
                    let first = self.held.remove(0);
                    if let Some(mut d) = self.scope.try_write() {
                        d.rate = self.sr;
                        d.column_secs = self.col_hop as f32 / self.sr;
                        d.push_column(first);
                    }
                }
                self.col_n = 0;
                self.col_peak = 0.0;
                self.col_thresh = 0.0;
                self.col_hit = 0.0;
            }
            if want_midi && self.note_on.is_some() {
                self.gate_left = self.gate_left.saturating_sub(1);
                if self.gate_left == 0
                    && let Some(note) = self.note_on.take()
                {
                    self.push_midi(i, [0x80, note, 0]);
                }
            }
            let mut out = 0.0;
            if want_audio && self.spike_left > 0 {
                out = self.spike;
                self.spike *= self.spike_decay;
                self.spike_left -= 1;
            }
            if listen {
                out = sc;
            }
            for c in io.iter_mut() {
                if let Some(v) = c.get_mut(i) {
                    *v = out;
                }
            }
        }
    }

    fn latency(&self) -> usize {
        self.lookahead
    }

    fn take_midi_out(&mut self, out: &mut Vec<(usize, [u8; 3])>) {
        for m in self.midi.drain(..) {
            if out.len() < out.capacity() {
                out.push(m);
            }
        }
    }

    fn scope(&self) -> Option<Arc<Scope>> {
        Some(Arc::clone(&self.scope))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_become_clicks_and_notes() {
        let sr = 48_000.0;
        let mut d = OpenDrt::new();
        d.prepare(sr, 512, 1);
        let mut x = vec![0.0f32; 48_000];
        for hit in [4_800usize, 19_200, 33_600] {
            for k in 0..2400 {
                x[hit + k] = 0.8 * (-(k as f32) / 300.0).exp() * (std::f32::consts::TAU * 80.0 * k as f32 / sr).sin();
            }
        }
        let mut notes = Vec::new();
        let mut clicks = 0;
        for (b, chunk) in x.chunks(512).enumerate() {
            let mut io = vec![chunk.to_vec()];
            d.process(&mut io, chunk.len());
            let mut m = Vec::with_capacity(64);
            d.take_midi_out(&mut m);
            notes.extend(m.into_iter().filter(|(_, msg)| msg[0] == 0x90).map(|(o, _)| b * 512 + o));
            clicks += io[0].windows(2).filter(|w| w[0] == 0.0 && w[1] > 0.0).count();
        }
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert_eq!(clicks, 3);
        let la = d.latency();
        for (n, want) in notes.iter().zip([4_800usize, 19_200, 33_600]) {
            assert!((*n as i64 - (want + la) as i64).abs() < 48, "{n} vs {}", want + la);
        }
    }
}
