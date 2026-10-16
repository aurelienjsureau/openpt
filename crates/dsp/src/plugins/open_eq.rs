use crate::params::{Params, choice, lin, log, param_plumbing, toggle};
use crate::scope::Scope;
use crate::{Category, ParamInfo, Plugin, PluginInfo, Unit};
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::f64::consts::PI;
use std::sync::Arc;

pub const BANDS: usize = 24;
const MAX_SEC: usize = 12;
pub const PER_BAND: usize = 12;

pub const BELL: usize = 0;
pub const LOW_SHELF: usize = 1;
pub const LOW_CUT: usize = 2;
pub const HIGH_SHELF: usize = 3;
pub const HIGH_CUT: usize = 4;
pub const NOTCH: usize = 5;
pub const BAND_PASS: usize = 6;
pub const TILT_SHELF: usize = 7;
pub const FLAT_TILT: usize = 8;

pub const STEREO: usize = 0;
pub const LEFT: usize = 1;
pub const RIGHT: usize = 2;
pub const MID: usize = 3;
pub const SIDE: usize = 4;

pub const SHAPES: &[&str] = &["Bell", "Low Shelf", "Low Cut", "High Shelf", "High Cut", "Notch", "Band Pass", "Tilt Shelf", "Flat Tilt"];
pub const SLOPE_DB: [u32; 9] = [6, 12, 18, 24, 30, 36, 48, 72, 96];
pub const SLOPES: &[&str] = &["6 dB/oct", "12 dB/oct", "18 dB/oct", "24 dB/oct", "30 dB/oct", "36 dB/oct", "48 dB/oct", "72 dB/oct", "96 dB/oct"];
pub const PLACES: &[&str] = &["L+R", "L", "R", "Mid", "Side"];
pub const PHASES: &[&str] = &["Zero Latency", "Linear Phase"];
pub const RESOLUTIONS: &[&str] = &["Low", "Medium", "High", "Very High", "Maximum"];

pub const K_USED: usize = 0;
pub const K_BYP: usize = 1;
pub const K_FREQ: usize = 2;
pub const K_GAIN: usize = 3;
pub const K_Q: usize = 4;
pub const K_SHAPE: usize = 5;
pub const K_SLOPE: usize = 6;
pub const K_PLACE: usize = 7;
pub const K_DYN: usize = 8;
pub const K_RANGE: usize = 9;
pub const K_THR: usize = 10;
pub const K_SC: usize = 11;
const G_OUT: usize = BANDS * PER_BAND;
const G_PHASE: usize = G_OUT + 1;
const G_RES: usize = G_OUT + 2;
const G_SOLO: usize = G_OUT + 3;

macro_rules! eq_bands {
    ($( $n:literal $f:literal ),*) => { [ $(
        toggle(concat!("b", $n, "used"), concat!("Band ", $n, " Used"), false),
        toggle(concat!("b", $n, "byp"), concat!("Band ", $n, " Bypass"), false),
        log(concat!("b", $n, "freq"), concat!("Band ", $n, " Frequency"), 10.0, 30000.0, $f, Unit::Hz),
        lin(concat!("b", $n, "gain"), concat!("Band ", $n, " Gain"), -30.0, 30.0, 0.0, Unit::Db),
        log(concat!("b", $n, "q"), concat!("Band ", $n, " Q"), 0.025, 40.0, 1.0, Unit::None),
        choice(concat!("b", $n, "shape"), concat!("Band ", $n, " Shape"), SHAPES, 0),
        choice(concat!("b", $n, "slope"), concat!("Band ", $n, " Slope"), SLOPES, 1),
        choice(concat!("b", $n, "place"), concat!("Band ", $n, " Stereo Placement"), PLACES, 0),
        toggle(concat!("b", $n, "dyn"), concat!("Band ", $n, " Dynamic"), false),
        lin(concat!("b", $n, "range"), concat!("Band ", $n, " Dynamic Range"), -30.0, 30.0, -6.0, Unit::Db),
        lin(concat!("b", $n, "thr"), concat!("Band ", $n, " Threshold"), -60.0, 0.0, -20.0, Unit::Db),
        toggle(concat!("b", $n, "sc"), concat!("Band ", $n, " External Side-Chain"), false),
    )*
        lin("out", "Output", -24.0, 24.0, 0.0, Unit::Db),
        choice("phase", "Processing Mode", PHASES, 0),
        choice("res", "Linear Phase Resolution", RESOLUTIONS, 1),
        lin("solo", "Band Solo", 0.0, 24.0, 0.0, Unit::None),
    ] };
}

static PARAMS: [ParamInfo; BANDS * PER_BAND + 4] = eq_bands!(
    1 30.0, 2 39.31, 3 51.5, 4 67.48, 5 88.41, 6 115.84, 7 151.78, 8 198.86, 9 260.55, 10 341.38, 11 447.29, 12 586.05,
    13 767.86, 14 1006.06, 15 1318.17, 16 1727.11, 17 2262.9, 18 2964.91, 19 3884.71, 20 5089.85, 21 6668.86, 22 8737.72,
    23 11448.4, 24 15000.0
);

pub static OPEN_EQ_INFO: PluginInfo = PluginInfo {
    id: "open_eq",
    name: "OpenEQ",
    short_name: "OpenEQ",
    category: Category::Eq,
    params: &PARAMS,
    is_instrument: false,
    audiosuite: true,
};

pub fn has_gain(s: usize) -> bool {
    matches!(s, BELL | LOW_SHELF | HIGH_SHELF | TILT_SHELF | FLAT_TILT)
}
pub fn has_slope(s: usize) -> bool {
    s != BELL && s != FLAT_TILT
}
pub fn is_cut(s: usize) -> bool {
    s == LOW_CUT || s == HIGH_CUT
}

#[derive(Debug, Clone, Copy)]
pub struct Sec {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Default for Sec {
    fn default() -> Self {
        Sec { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct St {
    z1: f64,
    z2: f64,
}

#[inline]
fn tick(s: &Sec, st: &mut St, x: f64) -> f64 {
    let y = s.b0 * x + st.z1;
    st.z1 = s.b1 * x - s.a1 * y + st.z2;
    st.z2 = s.b2 * x - s.a2 * y;
    y
}

struct SecPow {
    b0: f64,
    b1: f64,
    b2: f64,
    a0: f64,
    a1: f64,
    a2: f64,
}

impl SecPow {
    fn new(s: &Sec) -> SecPow {
        let sb = s.b0 + s.b1 + s.b2;
        let db = s.b0 - s.b1 + s.b2;
        let sa = 1.0 + s.a1 + s.a2;
        let da = 1.0 - s.a1 + s.a2;
        SecPow { b0: sb * sb, b1: db * db, b2: -4.0 * s.b0 * s.b2, a0: sa * sa, a1: da * da, a2: -4.0 * s.a2 }
    }
    #[inline]
    fn mag2(&self, p1: f64) -> f64 {
        let p0 = 1.0 - p1;
        let p2 = 4.0 * p0 * p1;
        let den = self.a0 * p0 + self.a1 * p1 + self.a2 * p2;
        (self.b0 * p0 + self.b1 * p1 + self.b2 * p2).max(0.0) / den.max(1.0e-300)
    }
}

fn matched2(n2: f64, n1: f64, n0: f64, d2: f64, d1: f64, d0: f64, w0: f64) -> Sec {
    let ha2 = |w: f64| {
        let x = w / w0;
        let x2 = x * x;
        let (nr, ni) = (n0 - n2 * x2, n1 * x);
        let (dr, di) = (d0 - d2 * x2, d1 * x);
        (nr * nr + ni * ni) / (dr * dr + di * di).max(1.0e-300)
    };
    let wp = w0 * (d0 / d2).sqrt();
    let qp = d1 / (2.0 * (d0 * d2).sqrt());
    let mut s = Sec::default();
    let r = (-qp * wp).exp();
    s.a1 = if qp <= 1.0 { -2.0 * r * ((1.0 - qp * qp).sqrt() * wp).min(0.98 * PI).cos() } else { -2.0 * r * ((qp * qp - 1.0).sqrt() * wp).cosh() };
    s.a2 = (-2.0 * qp * wp).exp();
    let a0 = (1.0 + s.a1 + s.a2).powi(2);
    let a1 = (1.0 - s.a1 + s.a2).powi(2);
    let a2 = -4.0 * s.a2;
    let wm = w0.min(0.9 * PI);
    let sh = (0.5 * wm).sin();
    let p1 = sh * sh;
    let p0 = 1.0 - p1;
    let p2 = 4.0 * p0 * p1;
    let bb0 = ha2(0.0) * a0;
    let bb1 = ha2(PI) * a1;
    let bb2 = (ha2(wm) * (a0 * p0 + a1 * p1 + a2 * p2) - bb0 * p0 - bb1 * p1) / p2;
    let (sb0, sb1) = (bb0.sqrt(), bb1.sqrt());
    let w = 0.5 * (sb0 + sb1);
    s.b0 = 0.5 * (w + (w * w + bb2).max(0.0).sqrt());
    s.b1 = 0.5 * (sb0 - sb1);
    s.b2 = if s.b0 > 1.0e-300 { -bb2 / (4.0 * s.b0) } else { 0.0 };
    s
}

fn matched1(n1: f64, n0: f64, d1: f64, d0: f64, w0: f64) -> Sec {
    let ha = |w: f64| {
        let x = w / w0;
        ((n0 * n0 + n1 * n1 * x * x) / (d0 * d0 + d1 * d1 * x * x)).sqrt()
    };
    let mut s = Sec { a1: -(-w0 * d0 / d1).exp(), ..Sec::default() };
    s.a2 = 0.0;
    let (h0, hn) = (ha(0.0), ha(PI));
    s.b0 = 0.5 * ((1.0 + s.a1) * h0 + (1.0 - s.a1) * hn);
    s.b1 = 0.5 * ((1.0 + s.a1) * h0 - (1.0 - s.a1) * hn);
    s.b2 = 0.0;
    s
}

#[derive(Debug, Clone, Copy)]
pub struct Design {
    n: usize,
    s: [Sec; MAX_SEC],
}

impl Default for Design {
    fn default() -> Self {
        Design { n: 0, s: [Sec::default(); MAX_SEC] }
    }
}

impl Design {
    fn add(&mut self, x: Sec) {
        if self.n < MAX_SEC {
            self.s[self.n] = x;
            self.n += 1;
        }
    }
    fn scale(&mut self, k: f64) {
        if self.n == 0 {
            self.add(Sec::default());
        }
        self.s[0].b0 *= k;
        self.s[0].b1 *= k;
        self.s[0].b2 *= k;
    }
    fn mag2(&self, p1: f64) -> f64 {
        self.s[..self.n].iter().map(|s| SecPow::new(s).mag2(p1)).product()
    }
    pub fn mag_db_at(&self, f: f64, fs: f64) -> f64 {
        let sh = (PI * f.min(0.5 * fs) / fs).sin();
        10.0 * self.mag2(sh * sh).max(1.0e-30).log10()
    }
}

fn butter_q(k: usize, order: usize) -> f64 {
    1.0 / (2.0 * ((2.0 * k as f64 - 1.0) * PI / (2.0 * order as f64)).sin())
}

const BUTTER_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;

fn shelf(d: &mut Design, high: bool, order: usize, gain: f64, q: f64, w0: f64) {
    let res = q / BUTTER_Q;
    for k in 1..=order / 2 {
        let qk = butter_q(k, order) * if k == 1 { res } else { 1.0 };
        let a = gain.powf(1.0 / order as f64);
        let sa = a.sqrt();
        if high {
            d.add(matched2(a * a, a * sa / qk, a, 1.0, sa / qk, a, w0));
        } else {
            d.add(matched2(a, a * sa / qk, a * a, a, sa / qk, 1.0, w0));
        }
    }
    if order % 2 == 1 {
        let g = gain.powf(1.0 / order as f64);
        let sg = g.sqrt();
        if high {
            d.add(matched1(g, sg, 1.0, sg, w0));
        } else {
            d.add(matched1(1.0, sg, 1.0, 1.0 / sg, w0));
        }
    }
}

fn cut(d: &mut Design, high: bool, order: usize, q: f64, w0: f64) {
    let res = q / BUTTER_Q;
    for k in 1..=order / 2 {
        let qk = butter_q(k, order) * if k == 1 { res } else { 1.0 };
        if high {
            d.add(matched2(1.0, 0.0, 0.0, 1.0, 1.0 / qk, 1.0, w0));
        } else {
            d.add(matched2(0.0, 0.0, 1.0, 1.0, 1.0 / qk, 1.0, w0));
        }
    }
    if order % 2 == 1 {
        if high {
            d.add(matched1(1.0, 0.0, 1.0, 1.0, w0));
        } else {
            d.add(matched1(0.0, 1.0, 1.0, 1.0, w0));
        }
    }
}

pub fn design(shape: usize, slope: usize, f: f64, q: f64, gain_db: f64, fs: f64) -> Design {
    let mut d = Design::default();
    let f = f.clamp(5.0, 0.499 * fs * 1.36);
    let q = q.clamp(0.025, 40.0);
    let w0 = 2.0 * PI * f / fs;
    let order = (SLOPE_DB[slope.min(SLOPE_DB.len() - 1)] / 6) as usize;
    let g = 10f64.powf(gain_db / 20.0);
    match shape {
        BELL => {
            let a = 10f64.powf(gain_db.abs() / 40.0);
            let mut s = matched2(1.0, a / q, 1.0, 1.0, 1.0 / (a * q), 1.0, w0);
            if gain_db < 0.0 {
                let b = s;
                s = Sec { b0: 1.0 / b.b0, b1: b.a1 / b.b0, b2: b.a2 / b.b0, a1: b.b1 / b.b0, a2: b.b2 / b.b0 };
                if !(s.a2.abs() < 0.9999 && s.a1.abs() < 1.0 + s.a2 - 1.0e-6) {
                    s = matched2(1.0, 1.0 / (a * q), 1.0, 1.0, a / q, 1.0, w0);
                }
            }
            d.add(s);
        }
        LOW_SHELF => shelf(&mut d, false, order, g, q, w0),
        HIGH_SHELF => shelf(&mut d, true, order, g, q, w0),
        TILT_SHELF => {
            shelf(&mut d, true, order, g, q, w0);
            d.scale(1.0 / g.sqrt());
        }
        LOW_CUT => cut(&mut d, true, order, q, w0),
        HIGH_CUT => cut(&mut d, false, order, q, w0),
        NOTCH | BAND_PASS => {
            let m = if shape == BAND_PASS { order.min(MAX_SEC) } else { (order / 2).max(1) };
            for _ in 0..m {
                d.add(if shape == NOTCH {
                    matched2(1.0, 0.0, 1.0, 1.0, 1.0 / q, 1.0, w0)
                } else {
                    matched2(0.0, 1.0 / q, 0.0, 1.0, 1.0 / q, 1.0, w0)
                });
            }
        }
        FLAT_TILT => {
            let gs = 10f64.powf(gain_db / 200.0);
            let sg = gs.sqrt();
            for i in 0..12 {
                let fi = 10.0 * 2f64.powf(i as f64 + 0.5);
                d.add(matched1(gs, sg, 1.0, sg, 2.0 * PI * fi / fs));
            }
            let at = d.mag_db_at(f, fs);
            d.scale(10f64.powf(-at / 20.0));
        }
        _ => {}
    }
    d
}

fn listen_filter(shape: usize, f: f64, q: f64, fs: f64) -> Sec {
    let f = f.clamp(10.0, 0.45 * fs);
    let q = q.clamp(0.1, 40.0);
    let w0 = 2.0 * PI * f / fs;
    match shape {
        LOW_SHELF | LOW_CUT => matched2(0.0, 0.0, 1.0, 1.0, 1.0 / BUTTER_Q, 1.0, w0),
        HIGH_SHELF | HIGH_CUT => matched2(1.0, 0.0, 0.0, 1.0, 1.0 / BUTTER_Q, 1.0, w0),
        TILT_SHELF | FLAT_TILT => Sec::default(),
        _ => matched2(0.0, 1.0 / q, 0.0, 1.0, 1.0 / q, 1.0, w0),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandSettings {
    pub active: bool,
    pub shape: usize,
    pub slope: usize,
    pub place: usize,
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
    pub dynamic: bool,
    pub range: f32,
    pub thr: f32,
    pub sc: bool,
}

impl Default for BandSettings {
    fn default() -> Self {
        BandSettings { active: false, shape: BELL, slope: 1, place: STEREO, freq: 1000.0, gain: 0.0, q: 1.0, dynamic: false, range: -6.0, thr: -20.0, sc: false }
    }
}

impl BandSettings {
    fn effective_gain(&self, delta: f32) -> f64 {
        if has_gain(self.shape) { self.gain as f64 + if self.dynamic { delta as f64 } else { 0.0 } } else { 0.0 }
    }
}

pub fn band_design(b: &BandSettings, extra_db: f32, fs: f64) -> Design {
    design(b.shape, b.slope, b.freq as f64, b.q as f64, b.effective_gain(0.0) + extra_db as f64, fs)
}

const CHUNK: usize = 32;
const RES_BASE: [usize; 5] = [2048, 4096, 8192, 16384, 32768];
const BANDS_PER_STEP: usize = 4;
const MAG_STEPS: usize = BANDS / BANDS_PER_STEP;

#[derive(Debug, Clone, Copy)]
struct Det {
    f: Sec,
    st: [St; 2],
    env: [f64; 2],
    shape: usize,
    fq: f32,
    qq: f32,
}

impl Default for Det {
    fn default() -> Self {
        Det { f: Sec::default(), st: [St::default(); 2], env: [0.0; 2], shape: usize::MAX, fq: 0.0, qq: 0.0 }
    }
}

#[derive(Debug, Clone, Copy)]
struct BandRt {
    live: bool,
    shape: usize,
    slope: usize,
    place: usize,
    lf: f64,
    g: f64,
    lq: f64,
    dlf: f64,
    dg: f64,
    dlq: f64,
    d: Design,
    st: [[St; MAX_SEC]; 2],
}

impl Default for BandRt {
    fn default() -> Self {
        BandRt {
            live: false,
            shape: usize::MAX,
            slope: usize::MAX,
            place: usize::MAX,
            lf: 0.0,
            g: 0.0,
            lq: 0.0,
            dlf: 1.0e9,
            dg: 1.0e9,
            dlq: 1.0e9,
            d: Design::default(),
            st: [[St::default(); MAX_SEC]; 2],
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct SoloRt {
    band: Option<usize>,
    f: Sec,
    st: [St; 2],
}

fn run_design(d: &Design, st: &mut [St; MAX_SEC], x: &mut [f64]) {
    for (s, z) in d.s[..d.n].iter().zip(st.iter_mut()) {
        for v in x.iter_mut() {
            *v = tick(s, z, *v);
        }
    }
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..60 {
        let t = x / (2.0 * k as f64);
        term *= t * t;
        sum += term;
        if term < 1.0e-14 * sum {
            break;
        }
    }
    sum
}

const LL: usize = 0;
const LR: usize = 1;
const RL: usize = 2;
const RR: usize = 3;

struct Linear {
    n: usize,
    h: usize,
    m: usize,
    fft_n: Arc<dyn Fft<f32>>,
    fft_m: Arc<dyn Fft<f32>>,
    ifft_m: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex<f32>>,
    hist: [Vec<f32>; 2],
    in_buf: [Vec<f32>; 2],
    out_buf: [Vec<f32>; 2],
    job_out: [Vec<f32>; 2],
    dly: [Vec<f32>; 2],
    win: Vec<f32>,
    x: [Vec<Complex<f32>>; 2],
    yc: [Vec<Complex<f32>>; 2],
    yo: [Vec<Complex<f32>>; 2],
    p1: Vec<f64>,
    comb: Vec<f64>,
    cbuf: Vec<Complex<f32>>,
    ker_cur: [Vec<Complex<f32>>; 4],
    ker_old: [Vec<Complex<f32>>; 4],
    cross_cur: bool,
    cross_old: bool,
    old_valid: bool,
    last_sig: [f32; BANDS * 7],
    last_len: usize,
    t: [Vec<f64>; 4],
    tmp: Vec<f64>,
    job_bands: [BandSettings; BANDS],
    job_delta: [f32; BANDS],
    job_active: bool,
    job_kernel: bool,
    job_fade: bool,
    job_ms: bool,
    job_step: usize,
    job_total: usize,
    pos: usize,
    dpos: usize,
}

impl Linear {
    fn new(res: usize, fs: f64) -> Linear {
        let mut mult = 1;
        while fs > 48_000.0 * mult as f64 * 1.5 {
            mult *= 2;
        }
        let n = RES_BASE[res.min(RES_BASE.len() - 1)] * mult;
        let (h, m) = (n / 2, 2 * n);
        let mut planner = FftPlanner::<f32>::new();
        let fft_n = planner.plan_fft_inverse(n);
        let fft_m = planner.plan_fft_forward(m);
        let ifft_m = planner.plan_fft_inverse(m);
        let scratch_len = fft_n.get_inplace_scratch_len().max(fft_m.get_inplace_scratch_len()).max(ifft_m.get_inplace_scratch_len());
        let beta = 10.0;
        let i0b = bessel_i0(beta);
        let win = (0..n)
            .map(|j| {
                let t = 2.0 * j as f64 / n as f64 - 1.0;
                (bessel_i0(beta * (1.0 - t * t).max(0.0).sqrt()) / i0b) as f32
            })
            .collect();
        let p1 = (0..=n / 2)
            .map(|k| {
                let s = (PI * k as f64 / n as f64).sin();
                s * s
            })
            .collect();
        let z = Complex::new(0.0, 0.0);
        let lat = n / 2 + 2 * h;
        let cv = || vec![z; m];
        Linear {
            n,
            h,
            m,
            fft_n,
            fft_m,
            ifft_m,
            scratch: vec![z; scratch_len],
            hist: [vec![0.0; m], vec![0.0; m]],
            in_buf: [vec![0.0; h], vec![0.0; h]],
            out_buf: [vec![0.0; h], vec![0.0; h]],
            job_out: [vec![0.0; h], vec![0.0; h]],
            dly: [vec![0.0; lat], vec![0.0; lat]],
            win,
            x: [cv(), cv()],
            yc: [cv(), cv()],
            yo: [cv(), cv()],
            p1,
            comb: vec![1.0; n / 2 + 1],
            cbuf: vec![z; n],
            ker_cur: [cv(), cv(), cv(), cv()],
            ker_old: [cv(), cv(), cv(), cv()],
            cross_cur: false,
            cross_old: false,
            old_valid: false,
            last_sig: [0.0; BANDS * 7],
            last_len: usize::MAX,
            t: [vec![1.0; n / 2 + 1], vec![1.0; n / 2 + 1], vec![1.0; n / 2 + 1], vec![1.0; n / 2 + 1]],
            tmp: vec![1.0; n / 2 + 1],
            job_bands: [BandSettings::default(); BANDS],
            job_delta: [0.0; BANDS],
            job_active: false,
            job_kernel: false,
            job_fade: false,
            job_ms: false,
            job_step: 0,
            job_total: 0,
            pos: 0,
            dpos: 0,
        }
    }

    fn latency(&self) -> usize {
        self.n / 2 + 2 * self.h
    }

    fn reset(&mut self) {
        for c in 0..2 {
            for v in [&mut self.hist[c], &mut self.in_buf[c], &mut self.out_buf[c], &mut self.job_out[c], &mut self.dly[c]] {
                v.iter_mut().for_each(|x| *x = 0.0);
            }
        }
        self.pos = 0;
        self.dpos = 0;
        self.last_len = usize::MAX;
        self.old_valid = false;
        self.job_active = false;
    }

    fn make_kernel(&mut self, which: Option<usize>, slot: usize) {
        let n = self.n;
        {
            let t = match which {
                Some(i) => &self.t[i],
                None => &self.comb,
            };
            for k in 0..=n / 2 {
                let v = Complex::new(t[k] as f32, 0.0);
                self.cbuf[k] = v;
                if k > 0 && k < n / 2 {
                    self.cbuf[n - k] = v;
                }
            }
        }
        self.fft_n.process_with_scratch(&mut self.cbuf, &mut self.scratch);
        let scale = 1.0 / n as f32;
        let ker = &mut self.ker_cur[slot];
        ker.iter_mut().for_each(|c| *c = Complex::new(0.0, 0.0));
        for j in 0..n {
            ker[j] = Complex::new(self.cbuf[(j + n / 2) % n].re * scale * self.win[j], 0.0);
        }
        self.fft_m.process_with_scratch(ker, &mut self.scratch);
    }

    fn convolve(&mut self, old: bool, out_ch: usize) {
        let (ker, cross) = if old { (&self.ker_old, self.cross_old) } else { (&self.ker_cur, self.cross_cur) };
        let y = if old { &mut self.yo[out_ch] } else { &mut self.yc[out_ch] };
        if !cross {
            let k = &ker[if out_ch == 0 { LL } else { RR }];
            for ((o, a), b) in y.iter_mut().zip(self.x[out_ch].iter()).zip(k.iter()) {
                *o = a * b;
            }
        } else {
            let (k0, k1) = if out_ch == 0 { (&ker[LL], &ker[LR]) } else { (&ker[RL], &ker[RR]) };
            for i in 0..self.m {
                y[i] = self.x[0][i] * k0[i] + self.x[1][i] * k1[i];
            }
        }
        self.ifft_m.process_with_scratch(y, &mut self.scratch);
    }

    fn start_job(&mut self, bs: &[BandSettings; BANDS], delta: &[f32; BANDS]) {
        let (h, m) = (self.h, self.m);
        for c in 0..2 {
            self.hist[c].copy_within(h..m, 0);
            let (dst, src) = (&mut self.hist[c][m - h..], &self.in_buf[c]);
            dst.copy_from_slice(src);
        }
        let mut sig = [0.0f32; BANDS * 7];
        let mut len = 0;
        self.job_ms = false;
        for b in 0..BANDS {
            self.job_bands[b] = bs[b];
            self.job_delta[b] = delta[b];
            let p = &self.job_bands[b];
            if !p.active {
                continue;
            }
            self.job_ms |= p.place == MID || p.place == SIDE;
            let v = [b as f32, p.shape as f32, p.slope as f32, p.place as f32, p.freq, p.q, p.effective_gain(delta[b]) as f32];
            sig[len..len + 7].copy_from_slice(&v);
            len += 7;
        }
        self.job_kernel = !(self.old_valid && len == self.last_len && sig[..len] == self.last_sig[..len]);
        self.job_fade = self.job_kernel && self.old_valid;
        if self.job_kernel {
            self.last_sig = sig;
            self.last_len = len;
        }
        self.job_total = 2 + if self.job_kernel { MAG_STEPS + if self.job_ms { 4 } else { 2 } } else { 0 } + 2 + if self.job_fade { 2 } else { 0 } + 1;
        self.job_step = 0;
        self.job_active = true;
    }

    fn mag_step(&mut self, s: usize, fs: f64) {
        let nb = self.n / 2 + 1;
        if s == 0 {
            for t in self.t.iter_mut() {
                t.iter_mut().for_each(|v| *v = 1.0);
            }
        }
        for b in s * BANDS_PER_STEP..(s + 1) * BANDS_PER_STEP {
            let p = self.job_bands[b];
            if !p.active {
                continue;
            }
            let d = design(p.shape, p.slope, p.freq as f64, p.q as f64, p.effective_gain(self.job_delta[b]), fs);
            self.tmp.iter_mut().for_each(|v| *v = 1.0);
            for sec in &d.s[..d.n] {
                let sp = SecPow::new(sec);
                for k in 0..nb {
                    self.tmp[k] *= sp.mag2(self.p1[k]);
                }
            }
            let targets: &[usize] = match p.place {
                LEFT => &[0],
                RIGHT => &[1],
                MID => &[2],
                SIDE => &[3],
                _ => &[0, 1],
            };
            for &tg in targets {
                for k in 0..nb {
                    self.t[tg][k] *= self.tmp[k];
                }
            }
        }
        if s == MAG_STEPS - 1 {
            for t in self.t.iter_mut() {
                t.iter_mut().for_each(|v| *v = v.sqrt());
            }
            if self.old_valid {
                std::mem::swap(&mut self.ker_cur, &mut self.ker_old);
                self.cross_old = self.cross_cur;
            }
            self.cross_cur = self.job_ms;
        }
    }

    fn kernel_step(&mut self, s: usize) {
        if !self.job_ms {
            self.make_kernel(Some(if s == 0 { 0 } else { 1 }), if s == 0 { LL } else { RR });
            return;
        }
        const LRI: [usize; 4] = [0, 1, 0, 1];
        const SGN: [f64; 4] = [1.0, -1.0, -1.0, 1.0];
        const SLOT: [usize; 4] = [LL, LR, RL, RR];
        let nb = self.n / 2 + 1;
        for k in 0..nb {
            self.comb[k] = self.t[LRI[s]][k] * 0.5 * (self.t[2][k] + SGN[s] * self.t[3][k]);
        }
        self.make_kernel(None, SLOT[s]);
    }

    fn step_job(&mut self, fs: f64) {
        let mut s = self.job_step;
        self.job_step += 1;
        if s < 2 {
            let x = &mut self.x[s];
            for (o, &v) in x.iter_mut().zip(self.hist[s].iter()) {
                *o = Complex::new(v, 0.0);
            }
            self.fft_m.process_with_scratch(x, &mut self.scratch);
            return;
        }
        s -= 2;
        if self.job_kernel {
            if s < MAG_STEPS {
                self.mag_step(s, fs);
                return;
            }
            s -= MAG_STEPS;
            let nk = if self.job_ms { 4 } else { 2 };
            if s < nk {
                self.kernel_step(s);
                return;
            }
            s -= nk;
        }
        if s < 2 {
            self.convolve(false, s);
            return;
        }
        s -= 2;
        if self.job_fade {
            if s < 2 {
                self.convolve(true, s);
                return;
            }
        }
        let (h, m) = (self.h, self.m);
        let scale = 1.0 / m as f32;
        for c in 0..2 {
            for i in 0..h {
                let cur = self.yc[c][m - h + i].re;
                self.job_out[c][i] = if self.job_fade {
                    let t = 0.5 - 0.5 * (std::f32::consts::PI * (i as f32 + 0.5) / h as f32).cos();
                    scale * (self.yo[c][m - h + i].re * (1.0 - t) + cur * t)
                } else {
                    scale * cur
                };
            }
        }
        if self.job_kernel {
            self.old_valid = true;
        }
        self.job_active = false;
    }
}

pub struct EqEngine {
    fs: f64,
    linear: Option<Box<Linear>>,
    res: usize,
    det: [Det; BANDS],
    band: [BandRt; BANDS],
    solo: SoloRt,
    pub delta: [f32; BANDS],
}

impl EqEngine {
    pub fn new(fs: f64) -> EqEngine {
        EqEngine { fs, linear: None, res: 1, det: [Det::default(); BANDS], band: [BandRt::default(); BANDS], solo: SoloRt::default(), delta: [0.0; BANDS] }
    }

    pub fn set_mode(&mut self, linear: bool, res: usize) {
        let res = res.min(RES_BASE.len() - 1);
        if linear == self.linear.is_some() && (!linear || res == self.res) {
            return;
        }
        self.res = res;
        self.linear = if linear { Some(Box::new(Linear::new(res, self.fs))) } else { None };
        self.reset();
    }

    pub fn latency(&self) -> usize {
        self.linear.as_ref().map_or(0, |l| l.latency())
    }

    pub fn reset(&mut self) {
        self.band = [BandRt::default(); BANDS];
        self.det = [Det::default(); BANDS];
        self.solo = SoloRt::default();
        if let Some(l) = &mut self.linear {
            l.reset();
        }
        self.delta = [0.0; BANDS];
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>, bs: &[BandSettings; BANDS], solo: Option<usize>) {
        let n = l.len().min(r.len());
        let mut off = 0;
        while off < n {
            let m = CHUNK.min(n - off);
            self.run_detectors(&l[off..off + m], &r[off..off + m], sc.map(|(a, b)| (&a[off..off + m], &b[off..off + m])), bs);
            let s = solo.filter(|&b| b < BANDS && bs[b].active);
            if self.linear.is_some() {
                self.linear_chunk(&mut l[off..off + m], &mut r[off..off + m], bs, s);
            } else {
                self.zl_chunk(&mut l[off..off + m], &mut r[off..off + m], bs, s);
            }
            off += m;
        }
    }

    fn run_detectors(&mut self, l: &[f32], r: &[f32], sc: Option<(&[f32], &[f32])>, bs: &[BandSettings; BANDS]) {
        let fs = self.fs;
        for b in 0..BANDS {
            let s = &bs[b];
            let d = &mut self.det[b];
            if !s.active || !s.dynamic || !has_gain(s.shape) {
                self.delta[b] = 0.0;
                d.env = [0.0; 2];
                continue;
            }
            if d.shape != s.shape || d.fq != s.freq || d.qq != s.q {
                d.f = listen_filter(s.shape, s.freq as f64, s.q as f64, fs);
                d.shape = s.shape;
                d.fq = s.freq;
                d.qq = s.q;
            }
            let (a, c) = match sc {
                Some(k) if s.sc => k,
                _ => (l, r),
            };
            let tr = (40.0 / (s.freq as f64).max(10.0)).clamp(0.04, 0.3);
            let rel = (-1.0 / (tr * fs)).exp();
            let nc = if s.place == STEREO { 2 } else { 1 };
            for i in 0..a.len().min(c.len()) {
                let (x0, x1) = (a[i] as f64, c[i] as f64);
                let x = match s.place {
                    LEFT => [x0, 0.0],
                    RIGHT => [x1, 0.0],
                    MID => [0.5 * (x0 + x1), 0.0],
                    SIDE => [0.5 * (x0 - x1), 0.0],
                    _ => [x0, x1],
                };
                for k in 0..nc {
                    let y = tick(&d.f, &mut d.st[k], x[k]).abs();
                    d.env[k] = if y > d.env[k] { y } else { y + rel * (d.env[k] - y) };
                }
            }
            let lvl = 20.0 * (d.env[0].max(d.env[1]) + 1.0e-30).log10();
            let excess = lvl - s.thr as f64;
            self.delta[b] = if excess > 0.0 { (s.range.abs() as f64).min(0.5 * excess).copysign(s.range as f64) as f32 } else { 0.0 };
        }
    }

    fn prepare_solo(&mut self, bs: &[BandSettings; BANDS], s: Option<usize>) {
        if s != self.solo.band {
            self.solo = SoloRt { band: s, ..SoloRt::default() };
        }
        if let Some(b) = s {
            self.solo.f = listen_filter(bs[b].shape, bs[b].freq as f64, bs[b].q as f64, self.fs);
        }
    }

    fn solo_sample(&mut self, place: usize, l: &mut f64, r: &mut f64) {
        let so = &mut self.solo;
        match place {
            LEFT => {
                *l = tick(&so.f, &mut so.st[0], *l);
                *r = 0.0;
            }
            RIGHT => {
                *r = tick(&so.f, &mut so.st[1], *r);
                *l = 0.0;
            }
            MID => {
                let v = tick(&so.f, &mut so.st[0], 0.5 * (*l + *r));
                *l = v;
                *r = v;
            }
            SIDE => {
                let v = tick(&so.f, &mut so.st[0], 0.5 * (*l - *r));
                *l = v;
                *r = -v;
            }
            _ => {
                *l = tick(&so.f, &mut so.st[0], *l);
                *r = tick(&so.f, &mut so.st[1], *r);
            }
        }
    }

    fn zl_chunk(&mut self, l: &mut [f32], r: &mut [f32], bs: &[BandSettings; BANDS], solo: Option<usize>) {
        let m = l.len();
        let fs = self.fs;
        let a = 1.0 - (-(m as f64) / (0.015 * fs)).exp();
        let mut any_ms = false;
        for b in 0..BANDS {
            let p = &bs[b];
            let k = &mut self.band[b];
            if !p.active {
                k.live = false;
                continue;
            }
            let tf = (p.freq as f64).ln();
            let tq = (p.q as f64).ln();
            let tg = p.effective_gain(self.delta[b]);
            if !k.live || k.shape != p.shape || k.slope != p.slope || k.place != p.place {
                let fade_in = !k.live && has_gain(p.shape);
                let g0 = if k.live && has_gain(k.shape) && has_gain(p.shape) { k.g } else { tg };
                *k = BandRt { live: true, shape: p.shape, slope: p.slope, place: p.place, lf: tf, lq: tq, g: if fade_in { 0.0 } else { g0 }, ..BandRt::default() };
            } else {
                let glide = |v: &mut f64, t: f64, eps: f64| {
                    *v += a * (t - *v);
                    if (t - *v).abs() < eps {
                        *v = t;
                    }
                };
                glide(&mut k.lf, tf, 1.0e-6);
                glide(&mut k.lq, tq, 1.0e-6);
                glide(&mut k.g, tg, 1.0e-5);
            }
            if (k.lf - k.dlf).abs() > 1.0e-7 || (k.g - k.dg).abs() > 1.0e-6 || (k.lq - k.dlq).abs() > 1.0e-7 {
                k.d = design(k.shape, k.slope, k.lf.exp(), k.lq.exp(), k.g, fs);
                k.dlf = k.lf;
                k.dg = k.g;
                k.dlq = k.lq;
            }
            any_ms |= k.place == MID || k.place == SIDE;
        }
        let mut xl = [0.0f64; CHUNK];
        let mut xr = [0.0f64; CHUNK];
        for i in 0..m {
            xl[i] = l[i] as f64;
            xr[i] = r[i] as f64;
        }
        if let Some(s) = solo {
            self.prepare_solo(bs, Some(s));
            for i in 0..m {
                let (mut a, mut b) = (xl[i], xr[i]);
                self.solo_sample(bs[s].place, &mut a, &mut b);
                l[i] = a as f32;
                r[i] = b as f32;
            }
            return;
        }
        self.prepare_solo(bs, None);
        for k in self.band.iter_mut() {
            if !k.live || k.place >= MID {
                continue;
            }
            let [s0, s1] = &mut k.st;
            if k.place != RIGHT {
                run_design(&k.d, s0, &mut xl[..m]);
            }
            if k.place != LEFT {
                run_design(&k.d, s1, &mut xr[..m]);
            }
        }
        if any_ms {
            for i in 0..m {
                let (mm, ss) = (0.5 * (xl[i] + xr[i]), 0.5 * (xl[i] - xr[i]));
                xl[i] = mm;
                xr[i] = ss;
            }
            for k in self.band.iter_mut() {
                if !k.live || k.place < MID {
                    continue;
                }
                let [s0, s1] = &mut k.st;
                if k.place == MID {
                    run_design(&k.d, s0, &mut xl[..m]);
                } else {
                    run_design(&k.d, s1, &mut xr[..m]);
                }
            }
            for i in 0..m {
                let (mm, ss) = (xl[i], xr[i]);
                xl[i] = mm + ss;
                xr[i] = mm - ss;
            }
        }
        for i in 0..m {
            l[i] = xl[i] as f32;
            r[i] = xr[i] as f32;
        }
    }

    fn linear_chunk(&mut self, l: &mut [f32], r: &mut [f32], bs: &[BandSettings; BANDS], solo: Option<usize>) {
        self.prepare_solo(bs, solo);
        let fs = self.fs;
        let delta = self.delta;
        let m = l.len();
        for i in 0..m {
            let Some(lin) = self.linear.as_deref_mut() else { return };
            let lat = lin.latency();
            let (dl, dr) = (lin.dly[0][lin.dpos], lin.dly[1][lin.dpos]);
            lin.dly[0][lin.dpos] = l[i];
            lin.dly[1][lin.dpos] = r[i];
            lin.dpos += 1;
            if lin.dpos == lat {
                lin.dpos = 0;
            }
            let (ol, or) = (lin.out_buf[0][lin.pos], lin.out_buf[1][lin.pos]);
            lin.in_buf[0][lin.pos] = l[i];
            lin.in_buf[1][lin.pos] = r[i];
            lin.pos += 1;
            let hop = lin.pos == lin.h;
            if hop {
                lin.pos = 0;
                while lin.job_active {
                    lin.step_job(fs);
                }
                for c in 0..2 {
                    std::mem::swap(&mut lin.out_buf[c], &mut lin.job_out[c]);
                }
                lin.start_job(bs, &delta);
            }
            if let Some(s) = solo {
                let (mut a, mut b) = (dl as f64, dr as f64);
                self.solo_sample(bs[s].place, &mut a, &mut b);
                l[i] = a as f32;
                r[i] = b as f32;
            } else {
                l[i] = ol;
                r[i] = or;
            }
        }
        if let Some(lin) = self.linear.as_deref_mut() {
            let target = (lin.job_total * lin.pos).div_ceil(lin.h);
            while lin.job_active && lin.job_step < target {
                lin.step_job(fs);
            }
        }
    }
}

pub struct OpenEq {
    p: Params,
    sr: f32,
    ch: usize,
    eng: EqEngine,
    set: [BandSettings; BANDS],
    gain: f32,
    work: [Vec<f32>; 2],
    pre: [Vec<f32>; 2],
    key: [Vec<f32>; 2],
    scope: Arc<Scope>,
}

impl Default for OpenEq {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenEq {
    pub fn new() -> OpenEq {
        OpenEq {
            p: Params::new(&PARAMS),
            sr: 48_000.0,
            ch: 2,
            eng: EqEngine::new(48_000.0),
            set: [BandSettings::default(); BANDS],
            gain: 1.0,
            work: [Vec::new(), Vec::new()],
            pre: [Vec::new(), Vec::new()],
            key: [Vec::new(), Vec::new()],
            scope: Scope::new(),
        }
    }

    fn update(&mut self) {}

    fn read_settings(&mut self, mono: bool) {
        for b in 0..BANDS {
            let i = b * PER_BAND;
            let s = &mut self.set[b];
            s.active = self.p.on(i + K_USED) && !self.p.on(i + K_BYP);
            s.shape = self.p.choice(i + K_SHAPE);
            s.slope = self.p.choice(i + K_SLOPE);
            s.place = self.p.choice(i + K_PLACE);
            s.freq = self.p.v(i + K_FREQ);
            s.gain = self.p.v(i + K_GAIN);
            s.q = self.p.v(i + K_Q);
            s.dynamic = self.p.on(i + K_DYN);
            s.range = self.p.v(i + K_RANGE);
            s.thr = self.p.v(i + K_THR);
            s.sc = self.p.on(i + K_SC);
            if mono && s.place != STEREO {
                if s.place == SIDE || s.place == RIGHT {
                    s.active = false;
                }
                s.place = STEREO;
            }
        }
    }

    fn run(&mut self, io: &mut [Vec<f32>], key: Option<&[Vec<f32>]>, frames: usize) {
        let nch = io.len().min(self.ch).min(2);
        if nch == 0 {
            return;
        }
        let frames = frames.min(io[..nch].iter().map(Vec::len).min().unwrap_or(0));
        self.eng.set_mode(self.p.choice(G_PHASE) == 1, self.p.choice(G_RES));
        self.read_settings(nch == 1);
        let solo = match self.p.v(G_SOLO).round() as usize {
            0 => None,
            k => Some(k - 1),
        };
        let target = crate::db_to_gain(self.p.v(G_OUT));
        let coef = 1.0 - (-1.0 / (0.05 * self.sr)).exp();
        let cap = self.work[0].len().max(1);
        let mut off = 0;
        while off < frames {
            let n = cap.min(frames - off);
            for c in 0..2 {
                let src = &io[c.min(nch - 1)][off..off + n];
                self.work[c][..n].copy_from_slice(src);
                self.pre[c][..n].copy_from_slice(src);
            }
            let has_key = key.is_some_and(|k| !k.is_empty());
            if let Some(k) = key.filter(|k| !k.is_empty()) {
                for c in 0..2 {
                    let src = &k[c.min(k.len() - 1)];
                    for (i, d) in self.key[c][..n].iter_mut().enumerate() {
                        *d = src.get(off + i).copied().unwrap_or(0.0);
                    }
                }
            }
            let [wl, wr] = &mut self.work;
            let [kl, kr] = &self.key;
            let sc = if has_key { Some((&kl[..n], &kr[..n])) } else { None };
            self.eng.process(&mut wl[..n], &mut wr[..n], sc, &self.set, solo);
            for i in 0..n {
                self.gain += coef * (target - self.gain);
                wl[i] *= self.gain;
                wr[i] *= self.gain;
            }
            for c in 0..nch {
                io[c][off..off + n].copy_from_slice(&self.work[c][..n]);
            }
            if let Some(mut d) = self.scope.try_write() {
                d.rate = self.sr;
                d.key_live = has_key;
                d.push_audio(&self.pre, n, false);
                d.push_audio(&self.work, n, true);
                d.bands[..BANDS].copy_from_slice(&self.eng.delta);
            }
            off += n;
        }
    }
}

impl Plugin for OpenEq {
    param_plumbing!(OPEN_EQ_INFO);

    fn prepare(&mut self, sample_rate: f32, max_block: usize, channels: usize) {
        self.sr = if sample_rate.is_finite() && sample_rate > 0.0 { sample_rate.clamp(8_000.0, 384_000.0) } else { 48_000.0 };
        self.ch = channels.clamp(1, 2);
        let n = max_block.clamp(1, 16_384);
        for v in self.work.iter_mut().chain(self.pre.iter_mut()).chain(self.key.iter_mut()) {
            *v = vec![0.0; n];
        }
        self.eng = EqEngine::new(self.sr as f64);
        self.eng.set_mode(self.p.choice(G_PHASE) == 1, self.p.choice(G_RES));
        self.gain = crate::db_to_gain(self.p.v(G_OUT));
    }

    fn reset(&mut self) {
        self.eng.reset();
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        self.run(io, None, frames);
    }

    fn accepts_key(&self) -> bool {
        true
    }

    fn process_keyed(&mut self, io: &mut [Vec<f32>], key: &[Vec<f32>], frames: usize) {
        self.run(io, Some(key), frames);
    }

    fn latency(&self) -> usize {
        if self.p.choice(G_PHASE) == 1 { Linear::latency_for(self.p.choice(G_RES), self.sr as f64) } else { 0 }
    }

    fn scope(&self) -> Option<Arc<Scope>> {
        Some(Arc::clone(&self.scope))
    }
}

impl Linear {
    fn latency_for(res: usize, fs: f64) -> usize {
        let mut mult = 1;
        while fs > 48_000.0 * mult as f64 * 1.5 {
            mult *= 2;
        }
        let n = RES_BASE[res.min(RES_BASE.len() - 1)] * mult;
        n / 2 + n
    }
}

pub fn band_from(get: impl Fn(&str) -> f32, b: usize) -> BandSettings {
    let i = b * PER_BAND;
    let v = |k: usize| get(PARAMS[i + k].id);
    BandSettings {
        active: v(K_USED) >= 0.5 && v(K_BYP) < 0.5,
        shape: v(K_SHAPE).round().max(0.0) as usize,
        slope: v(K_SLOPE).round().max(0.0) as usize,
        place: v(K_PLACE).round().max(0.0) as usize,
        freq: v(K_FREQ),
        gain: v(K_GAIN),
        q: v(K_Q),
        dynamic: v(K_DYN) >= 0.5,
        range: v(K_RANGE),
        thr: v(K_THR),
        sc: v(K_SC) >= 0.5,
    }
}

pub fn band_param(b: usize, k: usize) -> &'static str {
    PARAMS.get(b * PER_BAND + k).map_or("", |p| p.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (std::f32::consts::TAU * f * i as f32 / sr).sin() * 0.5).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn run(eq: &mut OpenEq, f: f32) -> f32 {
        let sr = 48_000.0;
        let x = sine(f, sr, 49_152);
        let io = vec![x.clone(), x];
        let mut out = vec![Vec::new(), Vec::new()];
        for start in (0..49_152).step_by(512) {
            let mut blk = vec![io[0][start..start + 512].to_vec(), io[1][start..start + 512].to_vec()];
            eq.process(&mut blk, 512);
            out[0].extend_from_slice(&blk[0]);
        }
        20.0 * (rms(&out[0][24_000..]) / rms(&sine(f, sr, 49_152)[24_000..])).log10()
    }

    #[test]
    fn bell_boosts_its_frequency_in_both_modes() {
        for phase in [0.0, 1.0] {
            let mut eq = OpenEq::new();
            eq.set_param("phase", phase);
            eq.set_param("b1used", 1.0);
            eq.set_param("b1freq", 1000.0);
            eq.set_param("b1gain", 6.0);
            eq.prepare(48_000.0, 512, 2);
            let at = run(&mut eq, 1000.0);
            assert!((at - 6.0).abs() < 0.2, "phase {phase}: {at}");
            let mut eq = OpenEq::new();
            eq.set_param("phase", phase);
            eq.set_param("b1used", 1.0);
            eq.set_param("b1freq", 1000.0);
            eq.set_param("b1gain", 6.0);
            eq.prepare(48_000.0, 512, 2);
            let off = run(&mut eq, 10_000.0);
            assert!(off.abs() < 0.3, "phase {phase}: {off}");
        }
    }

    #[test]
    fn cut_inverts_boost_and_low_cut_cuts() {
        let d = design(BELL, 1, 1000.0, 1.0, -9.0, 48_000.0);
        assert!((d.mag_db_at(1000.0, 48_000.0) + 9.0).abs() < 0.05);
        let c = design(LOW_CUT, 3, 1000.0, BUTTER_Q, 0.0, 48_000.0);
        assert!(c.mag_db_at(250.0, 48_000.0) < -40.0);
        assert!(c.mag_db_at(10_000.0, 48_000.0).abs() < 0.1);
    }
}
