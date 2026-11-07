use crate::params::{Params, choice, lin, log, param_plumbing, toggle};
use crate::scope::Scope;
use crate::{Category, ParamInfo, Plugin, PluginInfo, Unit};
use rustfft::num_complex::Complex32 as C32;
use std::f32::consts::PI;
use std::sync::Arc;

pub const BANDS: usize = 6;
pub const PER_BAND: usize = 8;
pub const TAPE: usize = 0;
pub const TUBE: usize = 1;
pub const PREAMP: usize = 2;
pub const TRANSFO: usize = 3;
pub const BELL: usize = 0;
pub const LOW_SHELF: usize = 1;
pub const HIGH_SHELF: usize = 2;
pub const POS_LR: usize = 0;
pub const POS_MID: usize = 1;
pub const POS_SIDES: usize = 2;

pub const TYPES: &[&str] = &["Tape", "Tube", "Preamp", "Transformer"];
pub const SHAPES: &[&str] = &["Bell", "Low Shelf", "High Shelf"];
pub const POSITIONS: &[&str] = &["L/R", "Mid", "Sides"];

pub const K_ON: usize = 0;
pub const K_BYP: usize = 1;
pub const K_TYPE: usize = 2;
pub const K_DRIVE: usize = 3;
pub const K_FREQ: usize = 4;
pub const K_Q: usize = 5;
pub const K_SHAPE: usize = 6;
pub const K_POS: usize = 7;
const G_GAIN: usize = BANDS * PER_BAND;
const G_MATCH: usize = G_GAIN + 1;

macro_rules! sat_bands {
    ($( $n:literal $f:literal ),*) => { [ $(
        toggle(concat!("b", $n, "on"), concat!("Band ", $n, " On"), false),
        toggle(concat!("b", $n, "byp"), concat!("Band ", $n, " Bypass"), false),
        choice(concat!("b", $n, "type"), concat!("Band ", $n, " Algo"), TYPES, 1),
        lin(concat!("b", $n, "drive"), concat!("Band ", $n, " Drive"), 0.0, 100.0, 0.0, Unit::Percent),
        log(concat!("b", $n, "freq"), concat!("Band ", $n, " Freq"), 20.0, 20000.0, $f, Unit::Hz),
        log(concat!("b", $n, "q"), concat!("Band ", $n, " Q"), 0.3, 12.0, 1.2, Unit::None),
        choice(concat!("b", $n, "shape"), concat!("Band ", $n, " Shape"), SHAPES, 0),
        choice(concat!("b", $n, "pos"), concat!("Band ", $n, " Position"), POSITIONS, 0),
    )*
        lin("gain", "Output", -24.0, 24.0, 0.0, Unit::Db),
        toggle("gainmatch", "Gain Match", true),
    ] };
}

static PARAMS: [ParamInfo; BANDS * PER_BAND + 2] = sat_bands!(1 120.0, 2 400.0, 3 1200.0, 4 3000.0, 5 7000.0, 6 12000.0);

pub static OPEN_SAT_INFO: PluginInfo = PluginInfo {
    id: "open_sat",
    name: "OpenSat",
    short_name: "OpenSat",
    category: Category::Harmonic,
    params: &PARAMS,
    is_instrument: false,
    audiosuite: true,
};

pub fn band_param(b: usize, k: usize) -> &'static str {
    PARAMS.get(b * PER_BAND + k).map_or("", |p| p.id)
}

#[derive(Debug, Clone, Copy, Default)]
struct Svf {
    shape: usize,
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: f32,
    ic2: f32,
}

impl Svf {
    fn set(&mut self, f: f32, q: f32, sr: f32, shape: usize) {
        let f = f.clamp(10.0, sr * 0.49);
        let g = (PI * f / sr).tan();
        self.shape = shape;
        self.k = 1.0 / q.max(0.05);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    #[inline]
    fn process(&mut self, v0: f32) -> f32 {
        let v3 = v0 - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        match self.shape {
            LOW_SHELF => v2,
            HIGH_SHELF => v0 - self.k * v1 - v2,
            _ => self.k * v1,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Allpass2 {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: f32,
    ic2: f32,
}

impl Allpass2 {
    fn set_g(&mut self, g: f32, q: f32) {
        self.k = 1.0 / q.max(0.05);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    #[inline]
    fn process(&mut self, v0: f32) -> f32 {
        let v3 = v0 - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        v0 - 2.0 * self.k * v1
    }
}

fn bandwidth_octaves(q: f32) -> f32 {
    ((2.0 / std::f32::consts::LN_2) * (1.0 / (2.0 * q.max(0.05))).asinh()).max(0.15)
}

const QB: f32 = std::f32::consts::FRAC_1_SQRT_2;

#[derive(Debug, Clone, Copy, Default)]
struct LrBand {
    lo1: [Svf; 2],
    hi1: [Svf; 2],
    lo2: [Svf; 2],
    hi2: [Svf; 2],
    comp: Allpass2,
    mid_lo: Allpass2,
    mid_hi: Allpass2,
    f_lo: f32,
    f_hi: f32,
    mode: usize,
}

fn edges_for(f: f32, q: f32, sr: f32, shape: usize) -> (f32, f32) {
    let nyq = sr * 0.49;
    if shape == LOW_SHELF || shape == HIGH_SHELF {
        let x = f.clamp(10.0, nyq);
        return (x, x);
    }
    let bw = 1.8 * bandwidth_octaves(q);
    ((f * 2f32.powf(-bw * 0.5)).clamp(10.0, nyq), (f * 2f32.powf(bw * 0.5)).clamp(10.0, nyq))
}

impl LrBand {
    fn update(&mut self, lo: f32, hi: f32, sr: f32) {
        self.f_lo = lo;
        self.f_hi = hi;
        for i in 0..2 {
            self.lo1[i].set(lo, QB, sr, LOW_SHELF);
            self.hi1[i].set(lo, QB, sr, HIGH_SHELF);
            self.lo2[i].set(hi, QB, sr, LOW_SHELF);
            self.hi2[i].set(hi, QB, sr, HIGH_SHELF);
        }
        let (g_lo, g_hi) = ((PI * lo / sr).tan(), (PI * hi / sr).tan());
        self.comp.set_g(g_hi, QB);
        self.mid_lo.set_g(g_lo, QB);
        self.mid_hi.set_g(g_hi, QB);
    }

    fn set(&mut self, f: f32, q: f32, sr: f32, shape: usize) {
        self.mode = shape;
        let (lo, hi) = edges_for(f, q, sr, shape);
        self.update(lo, hi, sr);
    }

    #[inline]
    fn process_mid(&mut self, x: f32) -> f32 {
        match self.mode {
            LOW_SHELF => self.mid_hi.process(x),
            HIGH_SHELF => self.mid_lo.process(x),
            _ => {
                let y = self.mid_lo.process(x);
                self.mid_hi.process(y)
            }
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> (f32, f32) {
        match self.mode {
            LOW_SHELF => {
                let (mut b, mut h) = (x, x);
                for i in 0..2 {
                    b = self.lo2[i].process(b);
                    h = self.hi2[i].process(h);
                }
                (b, h)
            }
            HIGH_SHELF => {
                let (mut b, mut l) = (x, x);
                for i in 0..2 {
                    b = self.hi1[i].process(b);
                    l = self.lo1[i].process(l);
                }
                (b, l)
            }
            _ => {
                let (mut low, mut up) = (x, x);
                for i in 0..2 {
                    low = self.lo1[i].process(low);
                    up = self.hi1[i].process(up);
                }
                let (mut b, mut h) = (up, up);
                for i in 0..2 {
                    b = self.lo2[i].process(b);
                    h = self.hi2[i].process(h);
                }
                (b, self.comp.process(low) + h)
            }
        }
    }
}

pub fn band_magnitude(f: f32, f0: f32, q: f32, sr: f32, shape: usize) -> f32 {
    let (lo, hi) = edges_for(f0, q, sr, shape);
    let tanw = |x: f32| (PI * x.clamp(1.0, sr * 0.499) / sr).tan();
    let gf = tanw(f);
    let sec = |fc: f32| {
        let r = gf / tanw(fc).max(1.0e-9);
        let den = C32::new(1.0 - r * r, r / QB);
        (C32::new(1.0, 0.0) / den, C32::new(-r * r, 0.0) / den)
    };
    let (_, hp_l) = sec(lo);
    let (lp_h, _) = sec(hi);
    let band = match shape {
        LOW_SHELF => lp_h * lp_h,
        HIGH_SHELF => hp_l * hp_l,
        _ => lp_h * lp_h * hp_l * hp_l,
    };
    band.norm()
}

fn log_cosh(x: f32) -> f32 {
    let a = x.abs();
    a + (-2.0 * a).exp().ln_1p() - std::f32::consts::LN_2
}

fn cubic_clip(x: f32) -> f32 {
    if x >= 1.0 {
        2.0 / 3.0
    } else if x <= -1.0 {
        -2.0 / 3.0
    } else {
        x - x * x * x / 3.0
    }
}

fn cubic_clip_ad(x: f32) -> f32 {
    if x > 1.0 {
        5.0 / 12.0 + (2.0 / 3.0) * (x - 1.0)
    } else if x < -1.0 {
        5.0 / 12.0 - (2.0 / 3.0) * (x + 1.0)
    } else {
        x * x * 0.5 - x * x * x * x / 12.0
    }
}

const TAPE_A: f32 = 2.60;
const TAPE_BIAS: f32 = 0.02;

fn langevin(u: f32) -> f32 {
    let x = u as f64;
    if x.abs() < 1.0e-3 {
        return (x / 3.0 - x * x * x / 45.0) as f32;
    }
    (1.0 / x.tanh() - 1.0 / x) as f32
}

fn ln_sinh_over_u(u: f32) -> f32 {
    let a = (u as f64).abs();
    if a < 1.0 {
        let a2 = a * a;
        return (a2 / 6.0 - a2 * a2 / 180.0 + a2 * a2 * a2 / 2835.0 - a2 * a2 * a2 * a2 / 37800.0) as f32;
    }
    (a - (2.0 * a).ln() + (-(-2.0 * a).exp()).ln_1p()) as f32
}

fn tape(x: f32) -> f32 {
    3.0 * TAPE_A * (langevin((x + TAPE_BIAS) / TAPE_A) - langevin(TAPE_BIAS / TAPE_A))
}

fn tape_ad(x: f32) -> f32 {
    3.0 * TAPE_A * (TAPE_A * ln_sinh_over_u((x + TAPE_BIAS) / TAPE_A) - langevin(TAPE_BIAS / TAPE_A) * x)
}

const VALVE_GRID: f32 = 0.50;
const VALVE_CLIP_POS: f32 = 2.20;
const VALVE_SAT_NEG: f32 = 1.50;
const VALVE_BLOCK: f32 = 1.00;
const VALVE_W: f32 = 1.5 * (VALVE_CLIP_POS - VALVE_GRID);

fn tube(x: f32) -> f32 {
    if x > VALVE_GRID {
        let v = x - VALVE_GRID;
        if v >= VALVE_W {
            return VALVE_CLIP_POS;
        }
        return VALVE_GRID + VALVE_W * cubic_clip(v / VALVE_W);
    }
    if x > 0.0 { x } else { VALVE_SAT_NEG * (x / VALVE_SAT_NEG).tanh() }
}

fn tube_ad(x: f32) -> f32 {
    let f_grid = VALVE_GRID * VALVE_GRID * 0.5;
    if x > VALVE_GRID {
        let v = x - VALVE_GRID;
        if v >= VALVE_W {
            let f_end = f_grid + VALVE_GRID * VALVE_W + VALVE_W * VALVE_W * cubic_clip_ad(1.0);
            return f_end + VALVE_CLIP_POS * (v - VALVE_W);
        }
        return f_grid + VALVE_GRID * v + VALVE_W * VALVE_W * cubic_clip_ad(v / VALVE_W);
    }
    if x > 0.0 { x * x * 0.5 } else { VALVE_SAT_NEG * VALVE_SAT_NEG * log_cosh(x / VALVE_SAT_NEG) }
}

fn grid_conduction(x: f32) -> f32 {
    if x <= VALVE_GRID {
        return x;
    }
    let d = x - VALVE_GRID;
    ((0.447_325_3 + 0.545_158_4 * (-0.324_158_4 * d).exp()) / 0.992_483_7) * x
}

const API_KNEE_P: f32 = 0.74;
const API_KNEE_N: f32 = 0.50;
const API_WIDTH: f32 = 0.20;

fn api_half(a: f32, knee: f32) -> f32 {
    if a <= knee {
        return a;
    }
    let w = API_WIDTH;
    if a >= knee + 2.0 * w {
        return knee + w;
    }
    let d = a - knee;
    a - d * d / (4.0 * w)
}

fn api_half_ad(a: f32, knee: f32) -> f32 {
    if a <= knee {
        return a * a * 0.5;
    }
    let w = API_WIDTH;
    let k2 = knee * knee * 0.5;
    if a <= knee + 2.0 * w {
        let d = a - knee;
        return k2 + knee * d + d * d * 0.5 - d * d * d / (12.0 * w);
    }
    let d2 = 2.0 * w;
    let f2 = k2 + knee * d2 + d2 * d2 * 0.5 - d2 * d2 * d2 / (12.0 * w);
    f2 + (knee + w) * (a - knee - d2)
}

fn preamp(x: f32) -> f32 {
    let y = api_half(x.abs(), if x >= 0.0 { API_KNEE_P } else { API_KNEE_N });
    if x < 0.0 { -y } else { y }
}

fn preamp_ad(x: f32) -> f32 {
    api_half_ad(x.abs(), if x >= 0.0 { API_KNEE_P } else { API_KNEE_N })
}

const IRON_K1: f32 = 0.60;
const IRON_K2: f32 = 1.30;
const IRON_S1: f32 = 0.28;
const IRON_S2: f32 = 0.05;

fn transfo(x: f32) -> f32 {
    let a = x.abs();
    let y = if a <= IRON_K1 {
        a
    } else if a <= IRON_K2 {
        IRON_K1 + IRON_S1 * (a - IRON_K1)
    } else {
        IRON_K1 + IRON_S1 * (IRON_K2 - IRON_K1) + IRON_S2 * (a - IRON_K2)
    };
    if x < 0.0 { -y } else { y }
}

fn transfo_ad(x: f32) -> f32 {
    let a = x.abs();
    let s1 = IRON_K1 * IRON_K1 * 0.5;
    let w1 = IRON_K2 - IRON_K1;
    if a <= IRON_K1 {
        return a * a * 0.5;
    }
    if a <= IRON_K2 {
        let u = a - IRON_K1;
        return s1 + IRON_K1 * u + IRON_S1 * u * u * 0.5;
    }
    let u = a - IRON_K2;
    s1 + IRON_K1 * w1 + IRON_S1 * w1 * w1 * 0.5 + (IRON_K1 + IRON_S1 * w1) * u + IRON_S2 * u * u * 0.5
}

pub fn shape(kind: usize, x: f32) -> f32 {
    match kind {
        TUBE => tube(x),
        PREAMP => preamp(x),
        TRANSFO => transfo(x),
        _ => tape(x),
    }
}

fn shape_ad(kind: usize, x: f32) -> f32 {
    match kind {
        TUBE => tube_ad(x),
        PREAMP => preamp_ad(x),
        TRANSFO => transfo_ad(x),
        _ => tape_ad(x),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Adaa {
    x1: f32,
    ad1: f32,
}

impl Adaa {
    #[inline]
    fn process(&mut self, kind: usize, x: f32) -> f32 {
        let d = x - self.x1;
        let ad = shape_ad(kind, x);
        let y = if d.abs() < 1.0e-4 { shape(kind, 0.5 * (x + self.x1)) } else { (ad - self.ad1) / d };
        self.x1 = x;
        self.ad1 = ad;
        y
    }
}

const FLUX_REF: f32 = 1000.0;

fn flux_tilt_db(f0: f32) -> f32 {
    (20.0 * (FLUX_REF / f0.max(10.0)).log10()).clamp(-20.0, 20.0)
}

fn drive_gain(percent: f32, kind: usize, f0: f32) -> f32 {
    let u = percent.clamp(0.0, 100.0) * 0.01;
    if kind == TAPE || kind == TUBE {
        return 10f32.powf(30.0 * u * 0.05);
    }
    let knee_db = if kind == PREAMP { 11.0 } else { 12.6 };
    let u_knee = 0.15;
    let db_max = if kind == PREAMP { 24.0 } else { 26.0 + flux_tilt_db(f0) };
    let db = if u <= u_knee { knee_db * u / u_knee } else { knee_db + (db_max - knee_db) * (u - u_knee) / (1.0 - u_knee) };
    10f32.powf(db * 0.05)
}

const SAT_REF: f32 = 0.10;
const NORM_RAMP: f32 = 0.30;

#[derive(Debug, Clone, Copy, Default)]
struct SatStage {
    band: LrBand,
    hf_active: bool,
    hf_a: f32,
    hf_z: f32,
    dc_r: f32,
    dc_x1: f32,
    dc_y1: f32,
    adaa: Adaa,
    env_sq: f32,
    out_sq: f32,
    env_coef: f32,
    norm: f32,
    makeup: f32,
    grid_bias: f32,
    gb_att: f32,
    gb_rel: f32,
    tick: u32,
}

impl SatStage {
    fn prepare(&mut self, sr: f32) {
        *self = SatStage { band: self.band, hf_active: self.hf_active, hf_a: self.hf_a, ..SatStage::default() };
        self.dc_r = 1.0 - std::f32::consts::TAU * 10.0 / sr;
        self.env_coef = 1.0 - (-1.0 / (0.25 * sr)).exp();
        self.gb_att = 1.0 - (-1.0 / (0.0015 * sr)).exp();
        self.gb_rel = 1.0 - (-1.0 / (0.0220 * sr)).exp();
        self.reset();
    }

    fn reset(&mut self) {
        for s in self.band.lo1.iter_mut().chain(self.band.hi1.iter_mut()).chain(self.band.lo2.iter_mut()).chain(self.band.hi2.iter_mut()) {
            s.ic1 = 0.0;
            s.ic2 = 0.0;
        }
        for a in [&mut self.band.comp, &mut self.band.mid_lo, &mut self.band.mid_hi] {
            a.ic1 = 0.0;
            a.ic2 = 0.0;
        }
        self.hf_z = 0.0;
        self.dc_x1 = 0.0;
        self.dc_y1 = 0.0;
        self.adaa = Adaa::default();
        self.env_sq = SAT_REF * SAT_REF;
        self.out_sq = SAT_REF * SAT_REF;
        self.norm = 1.0;
        self.makeup = 1.0;
        self.grid_bias = 0.0;
    }

    fn set_type(&mut self, kind: usize, sr: f32) {
        self.hf_active = kind == TAPE;
        self.hf_a = 1.0 - (-std::f32::consts::TAU * 13_000.0 / sr).exp();
    }

    #[inline]
    fn process(&mut self, x: f32, kind: usize, g: f32, drive_u: f32) -> f32 {
        let (b, rest) = self.band.process(x);
        self.env_sq += self.env_coef * (b * b - self.env_sq);
        self.tick += 1;
        if self.tick >= 64 {
            self.tick = 0;
            let env = self.env_sq.max(1.0e-10).sqrt();
            let r = (SAT_REF / env).clamp(0.05, 40.0);
            let e = (drive_u / NORM_RAMP).clamp(0.0, 1.0);
            self.norm = r.powf(e);
            let out = self.out_sq.max(1.0e-10).sqrt();
            self.makeup = (env / out).clamp(0.5, 16.0);
        }
        let gg = g * self.norm;
        let input = b * gg;
        let inv = 1.0 / gg.max(1.0e-6);
        let sat = if kind == TUBE {
            let drv = grid_conduction(input);
            let exc = (drv - VALVE_GRID).max(0.0);
            self.grid_bias += (if exc > self.grid_bias { self.gb_att } else { self.gb_rel }) * (exc - self.grid_bias);
            self.adaa.process(kind, drv - VALVE_BLOCK * self.grid_bias) * inv
        } else {
            self.adaa.process(kind, input) * inv
        };
        self.out_sq += self.env_coef * (sat * sat - self.out_sq);
        let wet = if kind == PREAMP || kind == TRANSFO { drive_u } else { 1.0 };
        let mut y = sat * self.makeup - b;
        if self.hf_active {
            self.hf_z += self.hf_a * (y - self.hf_z);
            y = self.hf_z;
        }
        let dc = y - self.dc_x1 + self.dc_r * self.dc_y1;
        self.dc_x1 = y;
        self.dc_y1 = dc;
        let shaped = dc + b;
        rest + wet * shaped + (1.0 - wet) * b
    }
}

#[derive(Debug, Clone)]
struct HalfBand {
    taps: Vec<f32>,
    buf: Vec<f32>,
    pos: usize,
}

impl HalfBand {
    fn new(len: usize) -> HalfBand {
        let c = (len - 1) / 2;
        let beta = 8.0f64;
        let i0 = |x: f64| {
            let (mut s, mut t) = (1.0, 1.0);
            for k in 1..50 {
                let h = x / (2.0 * k as f64);
                t *= h * h;
                s += t;
            }
            s
        };
        let taps = (0..len)
            .map(|i| {
                let n = i as f64 - c as f64;
                let sinc = if n == 0.0 { 0.5 } else { (std::f64::consts::PI * n * 0.5).sin() / (std::f64::consts::PI * n) };
                let r = n / c as f64;
                (sinc * i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0(beta)) as f32
            })
            .collect();
        HalfBand { taps, buf: vec![0.0; 2 * len], pos: 0 }
    }

    fn delay(&self) -> usize {
        (self.taps.len() - 1) / 2
    }

    fn reset(&mut self) {
        self.buf.iter_mut().for_each(|v| *v = 0.0);
        self.pos = 0;
    }

    #[inline]
    fn push(&mut self, x: f32) {
        let l = self.taps.len();
        self.pos = if self.pos == 0 { l - 1 } else { self.pos - 1 };
        self.buf[self.pos] = x;
        self.buf[self.pos + l] = x;
    }

    #[inline]
    fn out(&self) -> f32 {
        let l = self.taps.len();
        self.taps.iter().zip(&self.buf[self.pos..self.pos + l]).map(|(a, b)| a * b).sum()
    }
}

#[derive(Debug, Clone)]
struct Os4 {
    up1: HalfBand,
    up2: HalfBand,
    dn2: HalfBand,
    dn1: HalfBand,
}

impl Os4 {
    fn new() -> Os4 {
        Os4 { up1: HalfBand::new(63), up2: HalfBand::new(31), dn2: HalfBand::new(31), dn1: HalfBand::new(63) }
    }
    fn latency(&self) -> usize {
        ((self.up1.delay() + self.dn1.delay()) as f32 / 2.0 + (self.up2.delay() + self.dn2.delay()) as f32 / 4.0).round() as usize
    }
    fn reset(&mut self) {
        for h in [&mut self.up1, &mut self.up2, &mut self.dn2, &mut self.dn1] {
            h.reset();
        }
    }
    #[inline]
    fn up(&mut self, x: f32, out: &mut [f32; 4]) {
        let mut mid = [0.0f32; 2];
        self.up1.push(2.0 * x);
        mid[0] = self.up1.out();
        self.up1.push(0.0);
        mid[1] = self.up1.out();
        for (k, &m) in mid.iter().enumerate() {
            self.up2.push(2.0 * m);
            out[2 * k] = self.up2.out();
            self.up2.push(0.0);
            out[2 * k + 1] = self.up2.out();
        }
    }
    #[inline]
    fn down(&mut self, x: &[f32; 4]) -> f32 {
        let mut mid = [0.0f32; 2];
        for (k, m) in mid.iter_mut().enumerate() {
            self.dn2.push(x[2 * k]);
            self.dn2.push(x[2 * k + 1]);
            *m = self.dn2.out();
        }
        self.dn1.push(mid[0]);
        self.dn1.push(mid[1]);
        self.dn1.out()
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Ramp {
    cur: f32,
    target: f32,
    step: f32,
    left: u32,
}

impl Ramp {
    fn set(&mut self, target: f32, len: u32) {
        if ((target - self.target).abs() > 0.0 || self.left > 0)
            && target != self.target {
                self.target = target;
                self.left = len.max(1);
                self.step = (target - self.cur) / self.left as f32;
            }
    }
    fn snap(&mut self, v: f32) {
        *self = Ramp { cur: v, target: v, step: 0.0, left: 0 };
    }
    #[inline]
    fn next(&mut self) -> f32 {
        if self.left > 0 {
            self.left -= 1;
            self.cur = if self.left == 0 { self.target } else { self.cur + self.step };
        }
        self.cur
    }
    fn moving(&self) -> bool {
        self.left > 0
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Node {
    sat: [SatStage; 2],
    freq: Ramp,
    q: Ramp,
    drive: Ramp,
    drive_u: Ramp,
    kind: usize,
    last_kind: usize,
    pos: usize,
    shape: usize,
    running: bool,
}

pub struct OpenSat {
    p: Params,
    sr: f32,
    os_rate: f32,
    ch: usize,
    os: [Os4; 2],
    nodes: [Node; BANDS],
    coef_tick: u32,
    in_energy: f64,
    out_energy: f64,
    match_db: f32,
    out: Ramp,
    pre: [Vec<f32>; 2],
    scope: Arc<Scope>,
}

impl Default for OpenSat {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenSat {
    pub fn new() -> OpenSat {
        OpenSat {
            p: Params::new(&PARAMS),
            sr: 48_000.0,
            os_rate: 192_000.0,
            ch: 2,
            os: [Os4::new(), Os4::new()],
            nodes: [Node { last_kind: usize::MAX, ..Node::default() }; BANDS],
            coef_tick: 0,
            in_energy: 0.0,
            out_energy: 0.0,
            match_db: 0.0,
            out: Ramp { cur: 1.0, target: 1.0, step: 0.0, left: 0 },
            pre: [Vec::new(), Vec::new()],
            scope: Scope::new(),
        }
    }

    fn update(&mut self) {}

    fn setup_block(&mut self) {
        let osr = self.os_rate;
        let len = (0.02 * osr) as u32;
        for b in 0..BANDS {
            let i = b * PER_BAND;
            let on = self.p.on(i + K_ON) && !self.p.on(i + K_BYP);
            let kind = self.p.choice(i + K_TYPE);
            let drive = self.p.v(i + K_DRIVE);
            let freq = self.p.v(i + K_FREQ);
            let q = self.p.v(i + K_Q);
            let shape = self.p.choice(i + K_SHAPE);
            let n = &mut self.nodes[b];
            n.freq.set(freq, (0.03 * osr) as u32);
            n.q.set(q, (0.03 * osr) as u32);
            if !n.running && !n.drive.moving() {
                n.freq.snap(freq);
                n.q.snap(q);
            }
            n.shape = shape;
            for c in 0..2 {
                n.sat[c].band.set(n.freq.cur, n.q.cur, osr, shape);
                if kind != n.last_kind {
                    n.sat[c].set_type(kind, osr);
                }
            }
            n.last_kind = kind;
            n.kind = kind;
            n.drive.set(if on { drive_gain(drive, kind, freq) } else { 1.0 }, len);
            n.drive_u.set(if on { drive * 0.01 } else { 0.0 }, len);
            let was = n.running;
            n.running = on || (n.drive.moving() && n.drive.cur > 1.0001);
            n.pos = self.p.choice(i + K_POS);
            if was && !n.running {
                n.sat[0].reset();
                n.sat[1].reset();
            }
        }
    }
}

impl Plugin for OpenSat {
    param_plumbing!(OPEN_SAT_INFO);

    fn prepare(&mut self, sample_rate: f32, max_block: usize, channels: usize) {
        self.sr = if sample_rate.is_finite() && sample_rate > 0.0 { sample_rate.clamp(8_000.0, 384_000.0) } else { 48_000.0 };
        self.os_rate = self.sr * 4.0;
        self.ch = channels.clamp(1, 2);
        self.os = [Os4::new(), Os4::new()];
        for n in self.nodes.iter_mut() {
            *n = Node { last_kind: usize::MAX, ..Node::default() };
            n.drive.snap(1.0);
            for s in n.sat.iter_mut() {
                s.prepare(self.os_rate);
            }
        }
        let n = max_block.clamp(1, 16_384);
        self.pre = [vec![0.0; n], vec![0.0; n]];
        self.in_energy = 0.0;
        self.out_energy = 0.0;
        self.match_db = 0.0;
        self.out.snap(1.0);
    }

    fn reset(&mut self) {
        for o in self.os.iter_mut() {
            o.reset();
        }
        for n in self.nodes.iter_mut() {
            for s in n.sat.iter_mut() {
                s.reset();
            }
        }
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        let nch = io.len().min(self.ch).min(2);
        if nch == 0 {
            return;
        }
        let n = frames.min(io[..nch].iter().map(Vec::len).min().unwrap_or(0));
        let shown = n.min(self.pre[0].len());
        self.setup_block();
        for c in 0..2 {
            let src = &io[c.min(nch - 1)][..shown];
            self.pre[c][..shown].copy_from_slice(src);
        }
        let in_sum: f64 = io[..nch].iter().map(|c| c[..n].iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>()).sum();
        let osr = self.os_rate;
        let stereo = nch > 1;
        let any = self.nodes.iter().any(|x| x.running);
        for i in 0..n {
            let mut ul = [0.0f32; 4];
            let mut ur = [0.0f32; 4];
            self.os[0].up(io[0][i], &mut ul);
            if stereo {
                self.os[1].up(io[1][i], &mut ur);
            }
            if any {
                for k in 0..4 {
                    let mut dg = [1.0f32; BANDS];
                    let mut du = [0.0f32; BANDS];
                    for (b, nd) in self.nodes.iter_mut().enumerate() {
                        dg[b] = nd.drive.next();
                        du[b] = nd.drive_u.next();
                        nd.freq.next();
                        nd.q.next();
                    }
                    self.coef_tick += 1;
                    if self.coef_tick >= 32 {
                        self.coef_tick = 0;
                        for nd in self.nodes.iter_mut().filter(|x| x.running) {
                            let (lo, hi) = edges_for(nd.freq.cur, nd.q.cur, osr, nd.shape);
                            for s in nd.sat.iter_mut() {
                                s.band.update(lo, hi, osr);
                            }
                        }
                    }
                    let mut l = ul[k];
                    let mut r = if stereo { ur[k] } else { l };
                    for (b, nd) in self.nodes.iter_mut().enumerate() {
                        if nd.running && nd.pos == POS_LR {
                            l = nd.sat[0].process(l, nd.kind, dg[b], du[b]);
                            if stereo {
                                r = nd.sat[1].process(r, nd.kind, dg[b], du[b]);
                            }
                        }
                    }
                    if stereo {
                        let mut m = 0.5 * (l + r);
                        let mut s = 0.5 * (l - r);
                        for (b, nd) in self.nodes.iter_mut().enumerate() {
                            if nd.running && nd.pos == POS_MID {
                                m = nd.sat[0].process(m, nd.kind, dg[b], du[b]);
                                s = nd.sat[0].band.process_mid(s);
                            }
                        }
                        for (b, nd) in self.nodes.iter_mut().enumerate() {
                            if nd.running && nd.pos == POS_SIDES {
                                s = nd.sat[0].process(s, nd.kind, dg[b], du[b]);
                                m = nd.sat[0].band.process_mid(m);
                            }
                        }
                        l = m + s;
                        r = m - s;
                    } else {
                        for (b, nd) in self.nodes.iter_mut().enumerate() {
                            if nd.running && nd.pos == POS_MID {
                                l = nd.sat[0].process(l, nd.kind, dg[b], du[b]);
                            }
                        }
                    }
                    ul[k] = l;
                    ur[k] = r;
                }
            }
            io[0][i] = self.os[0].down(&ul);
            if stereo {
                io[1][i] = self.os[1].down(&ur);
            }
        }
        let out_sum: f64 = io[..nch].iter().map(|c| c[..n].iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>()).sum();
        let count = (n * nch).max(1) as f64;
        let coef = (-(n as f64) / (0.35 * self.sr as f64)).exp();
        self.in_energy = self.in_energy * coef + in_sum / count * (1.0 - coef);
        self.out_energy = self.out_energy * coef + out_sum / count * (1.0 - coef);
        if self.in_energy > 1.0e-9 && self.out_energy > 1.0e-12 {
            self.match_db = (10.0 * (self.in_energy / self.out_energy).log10() as f32).clamp(-18.0, 18.0);
        }
        let db = if self.p.on(G_MATCH) { self.match_db } else { self.p.v(G_GAIN) };
        self.out.set(crate::db_to_gain(db), (0.05 * self.sr) as u32);
        for i in 0..n {
            let g = self.out.next();
            for c in io[..nch].iter_mut() {
                c[i] *= g;
            }
        }
        if let Some(mut d) = self.scope.try_write() {
            d.rate = self.sr;
            d.readout = self.match_db;
            d.push_audio(&self.pre, shown, false);
            d.push_audio(&io[..nch], shown, true);
        }
    }

    fn latency(&self) -> usize {
        self.os[0].latency()
    }

    fn scope(&self) -> Option<Arc<Scope>> {
        Some(Arc::clone(&self.scope))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_passes_and_drive_adds_harmonics() {
        let sr = 48_000.0;
        let mut s = OpenSat::new();
        s.set_param("gainmatch", 0.0);
        s.prepare(sr, 512, 2);
        let lat = s.latency();
        let x: Vec<f32> = (0..4096).map(|i| 0.3 * (std::f32::consts::TAU * 440.0 * i as f32 / sr).sin()).collect();
        let mut io = vec![x[..2048].to_vec(), x[..2048].to_vec()];
        s.process(&mut io, 2048);
        let err: f32 = (1000..2000).map(|i| (io[0][i] - x[i - lat]).abs()).fold(0.0, f32::max);
        assert!(err < 0.03, "pass-through error {err}");
        let mut s = OpenSat::new();
        for (k, v) in [("b2on", 1.0), ("b2drive", 80.0), ("b2freq", 440.0), ("gainmatch", 0.0)] {
            s.set_param(k, v);
        }
        s.prepare(sr, 512, 2);
        let mut io = vec![x.clone(), x.clone()];
        s.process(&mut io, 4096);
        let diff: f32 = (3000..4000).map(|i| (io[0][i] - x[i - lat]).abs()).fold(0.0, f32::max);
        assert!(diff > 0.01, "drive changed nothing");
        assert!(io[0].iter().all(|v| v.is_finite()));
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    #[test]
    #[ignore]
    fn open_sat_cost() {
        for ch in [1usize, 2] {
            let mut p = OpenSat::new();
            for (k, v) in [("b1on", 1.0), ("b1drive", 43.0), ("b1freq", 436.0), ("b1type", 1.0), ("gainmatch", 1.0)] {
                p.set_param(k, v);
            }
            p.prepare(44_100.0, 1024, ch);
            let x: Vec<f32> = (0..1024).map(|i| 0.3 * (i as f32 * 0.05).sin()).collect();
            let mut io = vec![x.clone(); ch];
            let t = std::time::Instant::now();
            let n = 200;
            for _ in 0..n {
                p.process(&mut io, 1024);
                for c in io.iter_mut() {
                    c.copy_from_slice(&x);
                }
            }
            eprintln!("{ch} ch: {:.3} ms per 1024-sample block", t.elapsed().as_secs_f64() * 1000.0 / f64::from(n));
            let mut worst = 0.0f64;
            for _ in 0..400 {
                let mut z = vec![vec![0.0f32; 1024]; ch];
                let t = std::time::Instant::now();
                p.process(&mut z, 1024);
                worst = worst.max(t.elapsed().as_secs_f64() * 1000.0);
            }
            eprintln!("{ch} ch: worst {worst:.3} ms per block in the silence after it");
        }
    }
}
