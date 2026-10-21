use crate::biquad::{Biquad, FilterType};
use crate::params::{Params, choice, lin, log, param_plumbing};
use crate::util::{DelayLine, Rng, Smoother, flush, frames_in, ms_to_samples, sane_channels, sane_sr};
use crate::{Category, ParamInfo, Plugin, PluginInfo, Unit};

pub const ALGORITHMS: &[&str] = &["Hall", "Church", "Plate", "Room 1", "Room 2", "Ambient", "Nonlinear", "Dry Space"];
pub const SIZES: &[&str] = &["Small", "Medium", "Large"];
pub const NONLINEAR: usize = 6;
pub const DRY_SPACE: usize = 7;
const SIZE_SCALE: [f32; 3] = [0.62, 1.0, 1.45];

const P_ALGO: usize = 0;
const P_SIZE: usize = 1;
const P_INPUT: usize = 2;
const P_MIX: usize = 3;
const P_DECAY: usize = 4;
const P_PRE: usize = 5;
const P_DIFF: usize = 6;
const P_HF: usize = 7;
const P_LP: usize = 8;
const P_BALANCE: usize = 9;

const LINES: usize = 8;
const DIFFUSERS: usize = 4;
const MAX_PRE_MS: f32 = 300.0;
const MAX_TAIL_DELAY_MS: f32 = 40.0;
const MAX_ER_MS: f32 = 120.0;
const MAX_GATE_S: f32 = 1.0;
const MIN_GATE_S: f32 = 0.05;
const NL_TAPS: usize = 48;

static PARAMS: [ParamInfo; 10] = [
    choice("algorithm", "Algorithm", ALGORITHMS, 0),
    choice("size", "Size", SIZES, 1),
    lin("input", "Input", -48.0, 0.0, 0.0, Unit::Db),
    lin("mix", "Mix", 0.0, 100.0, 100.0, Unit::Percent),
    log("decay", "Decay", 0.1, 20.0, 2.4, Unit::Seconds),
    lin("pre_delay", "Pre-Delay", 0.0, MAX_PRE_MS, 0.0, Unit::Ms),
    lin("diffusion", "Diffusion", 0.0, 100.0, 70.0, Unit::Percent),
    log("hf_cut", "HF Cut", 1000.0, 20000.0, 8000.0, Unit::Hz),
    log("lp_filter", "LP Filter", 1000.0, 20000.0, 20000.0, Unit::Hz),
    lin("balance", "Early/Late", 0.0, 100.0, 50.0, Unit::Percent),
];

pub fn balance_gains(balance: f32) -> (f32, f32) {
    let b = (balance / 100.0).clamp(0.0, 1.0);
    ((2.0 * (1.0 - b)).min(1.0), (2.0 * b).min(1.0))
}

pub static OPEN_R_INFO: PluginInfo = PluginInfo {
    id: "open_r",
    name: "OpenR",
    short_name: "OpenR",
    category: Category::Reverb,
    params: &PARAMS,
    is_instrument: false,
    audiosuite: true,
};

pub struct Algo {
    lines_ms: [f32; LINES],
    diff_ms: [f32; DIFFUSERS],
    diff_g: f32,
    mod_ms: f32,
    mod_hz: f32,
    pub er: &'static [(f32, f32, f32)],
    pub er_level: f32,
    pub tail_level: f32,
    tone_hz: f32,
    pub decay: [f32; 3],
    pub tail_delay_ms: f32,
    er_smear: f32,
}

static ALGOS: [Algo; 8] = [
    Algo {
        lines_ms: [37.1, 43.7, 51.3, 59.9, 67.3, 76.1, 83.9, 97.3],
        diff_ms: [5.3, 7.9, 13.1, 17.9],
        diff_g: 0.72,
        mod_ms: 0.5,
        mod_hz: 0.35,
        er: &[
            (9.0, 0.5, -0.6),
            (14.0, 0.42, 0.7),
            (21.0, 0.38, -0.3),
            (27.0, 0.33, 0.5),
            (33.0, 0.3, -0.8),
            (41.0, 0.26, 0.2),
            (52.0, 0.22, 0.9),
            (63.0, 0.18, -0.5),
        ],
        er_level: 0.6,
        tail_level: 1.0,
        tone_hz: 12000.0,
        decay: [1.6, 2.4, 3.6],
        tail_delay_ms: 12.0,
        er_smear: 0.0,
    },
    Algo {
        lines_ms: [53.3, 61.7, 71.9, 83.3, 95.1, 107.9, 121.3, 137.9],
        diff_ms: [8.9, 13.7, 19.3, 27.1],
        diff_g: 0.68,
        mod_ms: 0.6,
        mod_hz: 0.25,
        er: &[(17.0, 0.45, -0.5), (29.0, 0.4, 0.6), (41.0, 0.35, -0.8), (56.0, 0.3, 0.3), (71.0, 0.26, -0.2), (89.0, 0.22, 0.8)],
        er_level: 0.55,
        tail_level: 1.0,
        tone_hz: 7000.0,
        decay: [2.6, 4.0, 6.0],
        tail_delay_ms: 25.0,
        er_smear: 0.0,
    },
    Algo {
        lines_ms: [13.71, 17.93, 21.07, 24.73, 28.31, 32.89, 36.13, 40.31],
        diff_ms: [3.13, 4.27, 7.93, 11.29],
        diff_g: 0.75,
        mod_ms: 0.4,
        mod_hz: 1.1,
        er: &[],
        er_level: 0.0,
        tail_level: 1.0,
        tone_hz: 16000.0,
        decay: [1.0, 1.8, 2.8],
        tail_delay_ms: 0.0,
        er_smear: 0.0,
    },
    Algo {
        lines_ms: [19.3, 23.9, 27.1, 31.7, 35.3, 39.1, 43.9, 47.3],
        diff_ms: [2.9, 4.1, 6.7, 9.1],
        diff_g: 0.7,
        mod_ms: 0.2,
        mod_hz: 0.8,
        er: &[
            (3.1, 0.7, -0.4),
            (5.3, 0.62, 0.5),
            (7.9, 0.55, -0.7),
            (10.7, 0.48, 0.3),
            (13.9, 0.42, 0.8),
            (17.3, 0.36, -0.2),
            (21.1, 0.3, 0.6),
            (25.7, 0.24, -0.6),
        ],
        er_level: 0.9,
        tail_level: 0.75,
        tone_hz: 11000.0,
        decay: [0.4, 0.7, 1.1],
        tail_delay_ms: 4.0,
        er_smear: 0.0,
    },
    Algo {
        lines_ms: [25.3, 29.9, 34.7, 39.1, 44.3, 49.7, 55.1, 61.3],
        diff_ms: [4.3, 6.1, 9.7, 13.3],
        diff_g: 0.7,
        mod_ms: 0.3,
        mod_hz: 0.6,
        er: &[
            (5.9, 0.65, -0.5),
            (9.1, 0.58, 0.6),
            (13.3, 0.5, -0.3),
            (17.9, 0.44, 0.8),
            (22.7, 0.38, -0.7),
            (28.1, 0.32, 0.2),
            (34.3, 0.26, -0.2),
            (41.9, 0.2, 0.5),
        ],
        er_level: 0.8,
        tail_level: 0.85,
        tone_hz: 8000.0,
        decay: [0.7, 1.1, 1.7],
        tail_delay_ms: 7.0,
        er_smear: 0.0,
    },
    Algo {
        lines_ms: [11.3, 14.9, 17.1, 19.7, 23.3, 26.9, 29.3, 33.1],
        diff_ms: [2.1, 3.7, 5.9, 8.3],
        diff_g: 0.78,
        mod_ms: 0.3,
        mod_hz: 0.9,
        er: &[
            (1.7, 0.6, -0.5),
            (2.9, 0.55, 0.5),
            (4.3, 0.5, -0.8),
            (6.1, 0.45, 0.8),
            (8.3, 0.4, -0.2),
            (10.9, 0.34, 0.3),
            (13.7, 0.28, -0.6),
            (16.9, 0.22, 0.6),
        ],
        er_level: 1.0,
        tail_level: 0.5,
        tone_hz: 10000.0,
        decay: [0.25, 0.4, 0.6],
        tail_delay_ms: 2.0,
        er_smear: 0.0,
    },
    Algo {
        lines_ms: [19.3, 23.9, 27.1, 31.7, 35.3, 39.1, 43.9, 47.3],
        diff_ms: [1.9, 3.1, 4.7, 6.3],
        diff_g: 0.7,
        mod_ms: 0.0,
        mod_hz: 0.0,
        er: &[],
        er_level: 0.0,
        tail_level: 1.0,
        tone_hz: 14000.0,
        decay: [0.15, 0.3, 0.5],
        tail_delay_ms: 0.0,
        er_smear: 0.0,
    },
    Algo {
        lines_ms: [9.7, 11.9, 13.3, 15.1, 17.3, 19.1, 21.7, 23.9],
        diff_ms: [1.3, 2.3, 3.7, 5.3],
        diff_g: 0.75,
        mod_ms: 0.15,
        mod_hz: 0.9,
        er: DRY_SPACE_ER,
        er_level: 1.0,
        tail_level: 0.3,
        tone_hz: 9000.0,
        decay: [0.15, 0.25, 0.4],
        tail_delay_ms: 6.0,
        er_smear: 0.6,
    },
];

static DRY_SPACE_ER: &[(f32, f32, f32)] = &[
    (0.9, 0.82, -0.3),
    (1.6, 0.78, 0.4),
    (2.3, 0.74, -0.7),
    (3.1, 0.71, 0.6),
    (3.8, 0.67, -0.1),
    (4.7, 0.64, 0.8),
    (5.5, 0.61, -0.5),
    (6.6, 0.58, 0.2),
    (7.6, 0.55, -0.8),
    (8.9, 0.52, 0.5),
    (10.1, 0.49, -0.3),
    (11.6, 0.46, 0.7),
    (13.0, 0.43, -0.6),
    (14.7, 0.4, 0.1),
    (16.5, 0.37, -0.9),
    (18.4, 0.34, 0.6),
    (20.6, 0.31, -0.2),
    (22.9, 0.28, 0.9),
    (25.4, 0.25, -0.6),
    (28.1, 0.22, 0.3),
    (31.0, 0.19, -0.4),
    (34.2, 0.16, 0.7),
    (37.6, 0.13, -0.8),
    (41.3, 0.1, 0.4),
];

pub fn algo(i: usize) -> &'static Algo {
    &ALGOS[i.min(ALGOS.len() - 1)]
}

pub fn size_scale(size: usize) -> f32 {
    SIZE_SCALE[size.min(2)]
}

pub fn default_decay(algorithm: usize, size: usize) -> f32 {
    algo(algorithm).decay[size.min(2)]
}

pub fn gate_secs(decay: f32) -> f32 {
    decay.clamp(MIN_GATE_S, MAX_GATE_S)
}

pub fn gate_shape(t: f32) -> f32 {
    if t < 0.83 { 1.0 - 0.15 * t } else { (1.0 - t) / 0.17 * 0.87 }
}

#[derive(Default)]
struct Allpass {
    buf: Vec<f32>,
    len: usize,
    pos: usize,
}

impl Allpass {
    fn allocate(&mut self, max: usize) {
        self.buf = vec![0.0; max.max(1)];
        self.len = self.len.clamp(1, self.buf.len());
        self.pos = 0;
    }

    fn set_len(&mut self, len: usize) {
        self.len = len.clamp(1, self.buf.len().max(1));
        self.pos %= self.len;
    }

    fn clear(&mut self) {
        self.buf.iter_mut().for_each(|v| *v = 0.0);
    }

    #[inline]
    fn process(&mut self, x: f32, g: f32) -> f32 {
        let Some(slot) = self.buf.get_mut(self.pos) else { return x };
        let d = *slot;
        let v = x + g * d;
        *slot = flush(v);
        self.pos += 1;
        if self.pos >= self.len {
            self.pos = 0;
        }
        d - g * v
    }
}

#[inline]
fn hadamard8(v: &mut [f32; LINES]) {
    let mut h = 1;
    while h < LINES {
        let mut i = 0;
        while i < LINES {
            for j in i..i + h {
                let (a, b) = (v[j], v[j + h]);
                v[j] = a + b;
                v[j + h] = a - b;
            }
            i += h * 2;
        }
        h *= 2;
    }
    let s = 1.0 / (LINES as f32).sqrt();
    v.iter_mut().for_each(|x| *x *= s);
}

pub struct OpenR {
    p: Params,
    sr: f32,
    ch: usize,
    layout: (usize, usize),
    pre: [DelayLine; 2],
    er_line: DelayLine,
    er_taps: Vec<(f32, f32, f32)>,
    er_ap: [[Allpass; 2]; 2],
    diff: [[Allpass; DIFFUSERS]; 2],
    lines: [DelayLine; LINES],
    line_len: [f32; LINES],
    lp: [f32; LINES],
    gains: [f32; LINES],
    damp: f32,
    nl: [DelayLine; 2],
    nl_taps: [[(f32, f32); NL_TAPS]; 2],
    out_lp: [Biquad; 2],
    lp_on: bool,
    input: Smoother,
    mix: Smoother,
    pre_ms: Smoother,
    early: Smoother,
    late: Smoother,
    lfo: f32,
}

impl Default for OpenR {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenR {
    pub fn new() -> OpenR {
        let mut r = OpenR {
            p: Params::new(&PARAMS),
            sr: 48_000.0,
            ch: 2,
            layout: (usize::MAX, usize::MAX),
            pre: Default::default(),
            er_line: DelayLine::default(),
            er_taps: Vec::with_capacity(32),
            er_ap: Default::default(),
            diff: Default::default(),
            lines: Default::default(),
            line_len: [1.0; LINES],
            lp: [0.0; LINES],
            gains: [0.0; LINES],
            damp: 0.0,
            nl: Default::default(),
            nl_taps: [[(1.0, 0.0); NL_TAPS]; 2],
            out_lp: [Biquad::default(); 2],
            lp_on: false,
            input: Smoother::new(1.0),
            mix: Smoother::new(1.0),
            pre_ms: Smoother::new(0.0),
            early: Smoother::new(1.0),
            late: Smoother::new(1.0),
            lfo: 0.0,
        };
        r.update();
        r.input.snap();
        r.mix.snap();
        r.pre_ms.snap();
        r.early.snap();
        r.late.snap();
        r
    }

    fn algo(&self) -> &'static Algo {
        algo(self.p.choice(P_ALGO))
    }

    fn update(&mut self) {
        let sr = self.sr;
        let layout = (self.p.choice(P_ALGO).min(ALGOS.len() - 1), self.p.choice(P_SIZE).min(2));
        if layout != self.layout {
            self.layout = layout;
            self.relayout();
        }
        let a = self.algo();
        self.input.set(crate::db_to_gain(self.p.v(P_INPUT)));
        self.mix.set(self.p.v(P_MIX) / 100.0);
        self.pre_ms.set(self.p.v(P_PRE));
        let (early, late) = balance_gains(self.p.v(P_BALANCE));
        self.early.set(early * a.er_level);
        self.late.set(late * a.tail_level);
        let cut = self.p.v(P_HF).min(a.tone_hz).min(sr * 0.45);
        self.damp = (-std::f32::consts::TAU * cut / sr).exp();
        let rt60 = self.p.v(P_DECAY).max(0.05);
        for (g, len) in self.gains.iter_mut().zip(self.line_len) {
            *g = 10f32.powf(-3.0 * len / sr / rt60).min(0.9999);
        }
        let lp = self.p.v(P_LP);
        self.lp_on = lp < 19_500.0;
        for b in &mut self.out_lp {
            b.set(FilterType::LowPass, lp.min(sr * 0.45), std::f32::consts::FRAC_1_SQRT_2, 0.0, sr);
        }
        if layout.0 == NONLINEAR {
            self.gate_taps();
        }
    }

    fn relayout(&mut self) {
        let (ai, si) = self.layout;
        let a = algo(ai);
        let s = size_scale(si);
        let sr = self.sr;
        for (len, ms) in self.line_len.iter_mut().zip(a.lines_ms) {
            *len = ms_to_samples(ms * s, sr).max(2.0);
        }
        for (side, aps) in self.diff.iter_mut().enumerate() {
            let spread = if side == 0 { 1.0 } else { 1.071 };
            for (ap, ms) in aps.iter_mut().zip(a.diff_ms) {
                ap.set_len(ms_to_samples(ms * s * spread, sr) as usize);
            }
        }
        for (side, aps) in self.er_ap.iter_mut().enumerate() {
            let spread = if side == 0 { 1.0 } else { 1.083 };
            for (ap, ms) in aps.iter_mut().zip([1.1, 1.9]) {
                ap.set_len(ms_to_samples(ms * s * spread, sr) as usize);
            }
        }
        self.er_taps.clear();
        let norm = 1.0 / a.er.iter().map(|t| t.1).sum::<f32>().max(1.0).sqrt();
        for &(ms, g, pan) in a.er {
            let (l, r) = (0.5 * (1.0 - pan), 0.5 * (1.0 + pan));
            self.er_taps.push((ms_to_samples(ms * s, sr), g * norm * l.sqrt() * 2f32.sqrt(), g * norm * r.sqrt() * 2f32.sqrt()));
        }
        self.clear();
    }

    fn gate_taps(&mut self) {
        let gate = gate_secs(self.p.v(P_DECAY)) * self.sr;
        let norm = 1.0 / (NL_TAPS as f32).sqrt();
        for (side, taps) in self.nl_taps.iter_mut().enumerate() {
            let mut rng = Rng::new(0x51ED_270B ^ (side as u32 + 1).wrapping_mul(0x9E37_79B9));
            for (k, tap) in taps.iter_mut().enumerate() {
                let t = (k as f32 + 0.5 + 0.4 * rng.bipolar()) / NL_TAPS as f32;
                let sign = if rng.next_u32() & 1 == 0 { 1.0 } else { -1.0 };
                *tap = ((t * gate).max(1.0), sign * gate_shape(t) * norm * 1.6);
            }
        }
    }

    fn clear(&mut self) {
        self.pre.iter_mut().for_each(DelayLine::clear);
        self.er_line.clear();
        self.diff.iter_mut().flatten().for_each(Allpass::clear);
        self.er_ap.iter_mut().flatten().for_each(Allpass::clear);
        self.lines.iter_mut().for_each(DelayLine::clear);
        self.nl.iter_mut().for_each(DelayLine::clear);
        self.lp = [0.0; LINES];
        self.out_lp.iter_mut().for_each(Biquad::reset);
    }
}

impl Plugin for OpenR {
    param_plumbing!(OPEN_R_INFO);

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = sane_sr(sample_rate);
        self.ch = sane_channels(channels);
        let sr = self.sr;
        let big = SIZE_SCALE[2];
        self.pre.iter_mut().for_each(|d| d.allocate(ms_to_samples(MAX_PRE_MS + MAX_TAIL_DELAY_MS * big, sr) as usize + 4));
        self.er_line.allocate(ms_to_samples(MAX_ER_MS * big, sr) as usize + 4);
        let max_diff = ALGOS.iter().flat_map(|a| a.diff_ms).fold(0.0, f32::max);
        for ap in self.diff.iter_mut().flatten() {
            ap.allocate(ms_to_samples(max_diff * big * 1.08, sr) as usize + 2);
        }
        for ap in self.er_ap.iter_mut().flatten() {
            ap.allocate(ms_to_samples(1.9 * big * 1.09, sr) as usize + 2);
        }
        let max_line = ALGOS.iter().flat_map(|a| a.lines_ms).fold(0.0, f32::max);
        let max_mod = ALGOS.iter().map(|a| a.mod_ms).fold(0.0, f32::max);
        for l in &mut self.lines {
            l.allocate((ms_to_samples(max_line * big + 2.0 * max_mod, sr) + 4.0) as usize);
        }
        self.nl.iter_mut().for_each(|d| d.allocate((MAX_GATE_S * sr) as usize + 4));
        self.input.set_time(30.0, sr);
        self.mix.set_time(30.0, sr);
        self.pre_ms.set_time(100.0, sr);
        self.early.set_time(30.0, sr);
        self.late.set_time(30.0, sr);
        self.layout = (usize::MAX, usize::MAX);
        self.update();
        self.reset();
    }

    fn reset(&mut self) {
        self.clear();
        self.input.snap();
        self.mix.snap();
        self.pre_ms.snap();
        self.early.snap();
        self.late.snap();
        self.lfo = 0.0;
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        let ch = self.ch.min(io.len());
        if ch == 0 {
            return;
        }
        let frames = frames_in(io, frames, ch);
        let sr = self.sr;
        let a = self.algo();
        let nonlinear = self.layout.0 == NONLINEAR;
        let g_diff = a.diff_g * self.p.v(P_DIFF) / 100.0;
        let mod_depth = ms_to_samples(a.mod_ms, sr);
        let lfo_inc = a.mod_hz / sr;
        let tail_delay = ms_to_samples(a.tail_delay_ms * size_scale(self.layout.1), sr);
        let damp = self.damp;
        let smear = a.er_smear * self.p.v(P_DIFF) / 100.0;
        for n in 0..frames {
            let gin = self.input.next();
            let xl = io.first().and_then(|b| b.get(n)).copied().filter(|v| v.is_finite()).unwrap_or(0.0) * gin;
            let xr = if ch > 1 { io.get(1).and_then(|b| b.get(n)).copied().filter(|v| v.is_finite()).unwrap_or(0.0) * gin } else { xl };
            let pre = ms_to_samples(self.pre_ms.next(), sr);
            let mut early = [0.0f32; 2];
            let mut ins = [xl, xr];
            for (side, x) in ins.iter_mut().enumerate() {
                let d = &mut self.pre[side];
                d.push(*x);
                early[side] = if pre < 1.0 { *x } else { d.read(pre) };
                let mut v = d.read((pre + tail_delay).max(1.0));
                if pre + tail_delay < 1.0 {
                    v = *x;
                }
                for ap in &mut self.diff[side] {
                    v = ap.process(v, g_diff);
                }
                *x = v;
            }
            let (mut erl, mut err) = (0.0f32, 0.0f32);
            if !self.er_taps.is_empty() {
                self.er_line.push(0.5 * (early[0] + early[1]));
                for &(d, gl, gr) in &self.er_taps {
                    let s = self.er_line.read(d);
                    erl += s * gl;
                    err += s * gr;
                }
                if smear > 0.0 {
                    for ap in &mut self.er_ap[0] {
                        erl = ap.process(erl, smear);
                    }
                    for ap in &mut self.er_ap[1] {
                        err = ap.process(err, smear);
                    }
                }
            }
            let (wl, wr) = if nonlinear {
                let mut out = [0.0f32; 2];
                for side in 0..2 {
                    self.nl[side].push(ins[side]);
                    let line = &self.nl[side];
                    out[side] = self.nl_taps[side].iter().map(|&(d, g)| line.read(d) * g).sum();
                }
                (out[0], out[1])
            } else {
                self.lfo += lfo_inc;
                if self.lfo >= 1.0 {
                    self.lfo -= 1.0;
                }
                let (s, c) = (self.lfo * std::f32::consts::TAU).sin_cos();
                let mods = [0.0, s, 0.0, c, 0.0, -s, 0.0, -c];
                let mut o = [0.0f32; LINES];
                for i in 0..LINES {
                    let raw = self.lines[i].read(self.line_len[i] + mod_depth * (1.0 + mods[i]));
                    self.lp[i] = flush(raw + (self.lp[i] - raw) * damp);
                    o[i] = raw;
                }
                let mut fb: [f32; LINES] = std::array::from_fn(|i| self.lp[i] * self.gains[i]);
                hadamard8(&mut fb);
                for (i, (line, f)) in self.lines.iter_mut().zip(fb).enumerate() {
                    line.push(flush(f + ins[i % 2] * 0.5));
                }
                (0.4 * (o[0] - o[2] + o[4] - o[6]), 0.4 * (o[1] - o[3] + o[5] - o[7]))
            };
            let (ge, gl) = (self.early.next(), self.late.next());
            let mut wet = [ge * erl + gl * wl, ge * err + gl * wr];
            if self.lp_on {
                for (w, f) in wet.iter_mut().zip(&mut self.out_lp) {
                    *w = f.process_sample(*w);
                }
            }
            let m = self.mix.next();
            for (c, b) in io.iter_mut().take(ch).enumerate() {
                let w = if ch == 1 { 0.5 * (wet[0] + wet[1]) } else { wet[c % 2] };
                if let Some(x) = b.get_mut(n) {
                    let dry = if x.is_finite() { *x } else { 0.0 };
                    *x = dry * (1.0 - m) + w * m;
                }
            }
        }
    }

    fn tail_samples(&self) -> usize {
        let a = self.algo();
        let body = if self.layout.0 == NONLINEAR { gate_secs(self.p.v(P_DECAY)) } else { self.p.v(P_DECAY) * 1.3 };
        let secs = body + (self.p.v(P_PRE) + a.tail_delay_ms * 1.5 + MAX_ER_MS * 1.5) * 0.001 + 0.1;
        (secs * self.sr) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse(algorithm: usize, size: usize, decay: f32, secs: f32) -> Vec<f32> {
        impulse_at(algorithm, size, decay, 50.0, secs)
    }

    fn impulse_at(algorithm: usize, size: usize, decay: f32, balance: f32, secs: f32) -> Vec<f32> {
        let mut r = OpenR::new();
        r.set_param("balance", balance);
        r.set_param("algorithm", algorithm as f32);
        r.set_param("size", size as f32);
        r.set_param("decay", decay);
        r.prepare(48_000.0, 512, 2);
        let total = (secs * 48_000.0) as usize;
        let mut out = Vec::with_capacity(total);
        let mut first = true;
        while out.len() < total {
            let mut io = vec![vec![0.0f32; 512], vec![0.0f32; 512]];
            if first {
                io[0][0] = 1.0;
                io[1][0] = 1.0;
                first = false;
            }
            r.process(&mut io, 512);
            assert!(io.iter().flatten().all(|v| v.is_finite()));
            out.extend(io[0].iter().zip(&io[1]).map(|(l, r)| 0.5 * (l + r)));
        }
        out
    }

    fn energy_db(x: &[f32], from_s: f32, to_s: f32) -> f32 {
        let (a, b) = ((from_s * 48_000.0) as usize, (to_s * 48_000.0) as usize);
        let e: f32 = x[a..b].iter().map(|v| v * v).sum::<f32>() / (b - a) as f32;
        10.0 * e.max(1.0e-30).log10()
    }

    #[test]
    fn the_tail_falls_about_sixty_db_over_the_decay() {
        for algorithm in [0, 1, 2, 3, 4] {
            let decay = 2.0;
            let ir = impulse(algorithm, 1, decay, 2.6);
            let drop = energy_db(&ir, 0.2, 0.3) - energy_db(&ir, 1.2, 1.3);
            assert!((24.0..44.0).contains(&drop), "algorithm {algorithm}: {drop:.1} dB");
        }
    }

    #[test]
    fn nonlinear_stops_at_the_gate() {
        let ir = impulse(NONLINEAR, 1, 0.3, 0.8);
        let during = energy_db(&ir, 0.05, 0.2);
        let after = energy_db(&ir, 0.4, 0.7);
        assert!(during > -60.0, "{during}");
        assert!(during - after > 50.0, "{during} {after}");
    }

    #[test]
    fn every_algorithm_and_size_is_audible_and_bounded() {
        for algorithm in 0..ALGORITHMS.len() {
            for size in 0..3 {
                let ir = impulse(algorithm, size, default_decay(algorithm, size), 1.0);
                let peak = ir.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(peak > 0.01 && peak < 2.0, "{algorithm}/{size}: {peak}");
            }
        }
    }

    #[test]
    fn early_late_takes_out_either_part() {
        let early_only = impulse_at(0, 1, 2.0, 0.0, 1.0);
        let late_only = impulse_at(0, 1, 2.0, 100.0, 1.0);
        let both = impulse_at(0, 1, 2.0, 50.0, 1.0);
        assert!(energy_db(&early_only, 0.3, 0.6) < -120.0, "early only: no tail");
        assert!(energy_db(&early_only, 0.005, 0.07) > -60.0);
        assert!(energy_db(&late_only, 0.0, 0.0115) < -120.0, "late only: nothing before the tail");
        assert!(energy_db(&both, 0.0, 0.0115) > -80.0);
        assert!(energy_db(&late_only, 0.3, 0.6) > -80.0);
    }

    #[test]
    fn dry_space_is_early_reflections_and_little_else() {
        let ir = impulse(DRY_SPACE, 1, default_decay(DRY_SPACE, 1), 1.0);
        let early = energy_db(&ir, 0.0, 0.05);
        let later = energy_db(&ir, 0.3, 0.6);
        assert!(early > -50.0, "{early}");
        assert!(early - later > 50.0, "{early} {later}");
    }

    #[test]
    fn switching_algorithms_while_playing_stays_finite() {
        let mut r = OpenR::new();
        r.prepare(44_100.0, 256, 1);
        for k in 0..40 {
            r.set_param("algorithm", (k % ALGORITHMS.len()) as f32);
            r.set_param("size", (k % 3) as f32);
            let mut io = vec![(0..256).map(|i| ((i + k) as f32 * 0.1).sin()).collect::<Vec<_>>()];
            r.process(&mut io, 256);
            assert!(io[0].iter().all(|v| v.is_finite()));
        }
    }
}
