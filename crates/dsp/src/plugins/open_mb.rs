use super::linmb_table as linmb;
use crate::params::{Params, choice, lin, log, param_plumbing, toggle};
use crate::scope::Scope;
use crate::{Category, ParamInfo, Plugin, PluginInfo, Unit};
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::f64::consts::PI;
use std::sync::Arc;

pub const BANDS: usize = 6;
pub const PER_BAND: usize = 20;
pub const MIN_F: f32 = 10.0;
pub const MAX_F: f32 = 30_000.0;
pub const OPEN_LO: f32 = 10.5;
pub const OPEN_HI: f32 = 29_000.0;

pub const STEREO: usize = 0;
pub const LEFT: usize = 1;
pub const RIGHT: usize = 2;
pub const MID: usize = 3;
pub const SIDE: usize = 4;
pub const LAW_RATIO: usize = 0;
pub const LAW_RANGE: usize = 1;

pub const PLACES: &[&str] = &["L+R", "L", "R", "Mid", "Side"];
pub const PHASES: &[&str] = &["Dynamic Phase", "Linear Phase"];
pub const MODES: &[&str] = &["Compress", "Expand"];
pub const LAWS: &[&str] = &["Ratio", "Range"];

pub const K_USED: usize = 0;
pub const K_BYP: usize = 1;
pub const K_SOLO: usize = 2;
pub const K_LO: usize = 3;
pub const K_HI: usize = 4;
pub const K_SLO: usize = 5;
pub const K_SHI: usize = 6;
pub const K_MODE: usize = 7;
pub const K_THR: usize = 8;
pub const K_RANGE: usize = 9;
pub const K_RATIO: usize = 10;
pub const K_KNEE: usize = 11;
pub const K_ATK: usize = 12;
pub const K_REL: usize = 13;
pub const K_LA: usize = 14;
pub const K_LEVEL: usize = 15;
pub const K_LATK: usize = 16;
pub const K_LREL: usize = 17;
pub const K_PLACE: usize = 18;
pub const K_SC: usize = 19;
const G: usize = BANDS * PER_BAND;
const G_PHASE: usize = G;
const G_LOOKAHEAD: usize = G + 1;
const G_LAW: usize = G + 2;
const G_LKNEE: usize = G + 3;
const G_RELMODE: usize = G + 4;
const G_BEHAVIOR: usize = G + 5;
const G_MIX: usize = G + 6;
const G_OUT: usize = G + 7;

macro_rules! mb_bands {
    ($( $n:literal $lo:literal $hi:literal ),*) => { [ $(
        toggle(concat!("b", $n, "used"), concat!("Band ", $n, " Used"), false),
        toggle(concat!("b", $n, "byp"), concat!("Band ", $n, " Bypass"), false),
        toggle(concat!("b", $n, "solo"), concat!("Band ", $n, " Solo"), false),
        log(concat!("b", $n, "lo"), concat!("Band ", $n, " Low Crossover"), 10.0, 30000.0, $lo, Unit::Hz),
        log(concat!("b", $n, "hi"), concat!("Band ", $n, " High Crossover"), 10.0, 30000.0, $hi, Unit::Hz),
        lin(concat!("b", $n, "slo"), concat!("Band ", $n, " Low Slope"), 6.0, 48.0, 24.0, Unit::None),
        lin(concat!("b", $n, "shi"), concat!("Band ", $n, " High Slope"), 6.0, 48.0, 24.0, Unit::None),
        choice(concat!("b", $n, "mode"), concat!("Band ", $n, " Dynamics Mode"), MODES, 0),
        lin(concat!("b", $n, "thr"), concat!("Band ", $n, " Threshold"), -90.0, 0.0, -18.0, Unit::Db),
        lin(concat!("b", $n, "range"), concat!("Band ", $n, " Range"), -30.0, 30.0, -6.0, Unit::Db),
        log(concat!("b", $n, "ratio"), concat!("Band ", $n, " Ratio"), 1.0, 100.0, 4.0, Unit::Ratio),
        lin(concat!("b", $n, "knee"), concat!("Band ", $n, " Knee"), 0.0, 48.0, 24.0, Unit::Db),
        lin(concat!("b", $n, "atk"), concat!("Band ", $n, " Attack"), 0.0, 100.0, 20.0, Unit::Percent),
        lin(concat!("b", $n, "rel"), concat!("Band ", $n, " Release"), 0.0, 100.0, 20.0, Unit::Percent),
        lin(concat!("b", $n, "la"), concat!("Band ", $n, " Lookahead"), 0.0, 20.0, 1.0, Unit::Ms),
        lin(concat!("b", $n, "level"), concat!("Band ", $n, " Level"), -30.0, 30.0, 0.0, Unit::Db),
        log(concat!("b", $n, "latk"), concat!("Band ", $n, " Range Attack"), 0.5, 500.0, 16.03, Unit::Ms),
        log(concat!("b", $n, "lrel"), concat!("Band ", $n, " Range Release"), 5.0, 5000.0, 25.0, Unit::Ms),
        choice(concat!("b", $n, "place"), concat!("Band ", $n, " Stereo Placement"), PLACES, 0),
        toggle(concat!("b", $n, "sc"), concat!("Band ", $n, " External Side-Chain"), false),
    )*
        choice("phase", "Processing Mode", PHASES, 0),
        toggle("lookahead", "Lookahead Enabled", true),
        choice("law", "Dynamics Law", LAWS, 0),
        lin("lknee", "Range Knee", 0.0, 0.75, 0.5, Unit::None),
        choice("relmode", "Range Release Mode", &["ARC", "Manual"], 0),
        choice("behavior", "Range Behavior", &["Electro", "Opto"], 0),
        lin("mix", "Mix", 0.0, 200.0, 100.0, Unit::Percent),
        lin("out", "Output", -24.0, 24.0, 0.0, Unit::Db),
    ] };
}

static PARAMS: [ParamInfo; BANDS * PER_BAND + 8] =
    mb_bands!(1 30.0 120.0, 2 90.94 363.77, 3 275.69 1102.75, 4 835.73 3342.91, 5 2533.46 10133.82, 6 7680.0 30000.0);

pub static OPEN_MB_INFO: PluginInfo = PluginInfo {
    id: "open_mb",
    name: "OpenMB",
    short_name: "OpenMB",
    category: Category::Dynamics,
    params: &PARAMS,
    is_instrument: false,
    audiosuite: true,
};

pub fn band_param(b: usize, k: usize) -> &'static str {
    PARAMS.get(b * PER_BAND + k).map_or("", |p| p.id)
}

pub fn presets() -> Vec<(&'static str, Vec<(&'static str, f32)>)> {
    let mut v = Vec::new();
    const XO: [f32; 6] = [MIN_F, 92.0, 545.0, 4000.0, 11071.0, MAX_F];
    const ATK: [f32; 5] = [34.99, 22.18, 16.03, 4.99, 2.0];
    const REL: [f32; 5] = [80.9, 40.27, 25.0, 9.98, 5.0];
    for b in 0..5 {
        for (k, x) in [
            (K_USED, 1.0),
            (K_LO, XO[b]),
            (K_HI, XO[b + 1]),
            (K_SLO, 28.8),
            (K_SHI, 28.8),
            (K_THR, 0.0),
            (K_RANGE, -6.0),
            (K_LATK, ATK[b]),
            (K_LREL, REL[b]),
        ] {
            v.push((band_param(b, k), x));
        }
    }
    v.extend([("phase", 1.0), ("lookahead", 0.0), ("law", LAW_RANGE as f32)]);
    vec![("LinMB Default", v)]
}

type C64 = Complex<f64>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub lo: f64,
    pub hi: f64,
    pub k_lo: f64,
    pub k_hi: f64,
    pub has_lo: bool,
    pub has_hi: bool,
}

impl Default for Shape {
    fn default() -> Self {
        Shape { lo: 100.0, hi: 1000.0, k_lo: 4.0, k_hi: 4.0, has_lo: true, has_hi: true }
    }
}

fn edge_low(f: f64, fc: f64, k: f64) -> f64 {
    let u = (f / fc).powf(k);
    u / (1.0 + u)
}
fn edge_high(f: f64, fc: f64, k: f64) -> f64 {
    1.0 / (1.0 + (f / fc).powf(k))
}

pub fn band_mag(s: &Shape, f: f64) -> f64 {
    let mut b = 1.0;
    if s.has_lo {
        b *= edge_low(f, s.lo, s.k_lo);
    }
    if s.has_hi {
        b *= edge_high(f, s.hi, s.k_hi);
    }
    b
}

fn even_order(k: f64) -> f64 {
    (2 * ((k * 0.5 - 1.0e-9).ceil() as i64)).max(2) as f64
}

fn even_shape(mut s: Shape) -> Shape {
    s.k_lo = even_order(s.k_lo);
    s.k_hi = even_order(s.k_hi);
    s
}

fn geo_centre(s: &Shape) -> f64 {
    let a = if s.has_lo { s.lo } else { 20.0 };
    let b = if s.has_hi { s.hi } else { 20_000.0 };
    (a * b).sqrt()
}

#[derive(Debug, Clone, Copy)]
struct Bq {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Default for Bq {
    fn default() -> Self {
        Bq { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct BqSt {
    z1: f64,
    z2: f64,
}

const MAX_SEC: usize = 8;

#[derive(Debug, Clone, Copy, Default)]
struct Cascade {
    s: [Bq; MAX_SEC],
    n: usize,
}

impl Cascade {
    #[inline]
    fn tick(&self, st: &mut [BqSt; MAX_SEC], mut x: f64) -> f64 {
        for (q, s) in self.s[..self.n].iter().zip(st.iter_mut()) {
            let y = q.b0 * x + s.z1;
            s.z1 = q.b1 * x - q.a1 * y + s.z2;
            s.z2 = q.b2 * x - q.a2 * y;
            x = y;
        }
        x
    }
    fn mag_at(&self, f: f64, fs: f64) -> f64 {
        let z1 = C64::from_polar(1.0, -2.0 * PI * f / fs);
        let z2 = z1 * z1;
        let mut h = C64::new(1.0, 0.0);
        for s in &self.s[..self.n] {
            h *= (z1 * s.b1 + z2 * s.b2 + s.b0) / (z1 * s.a1 + z2 * s.a2 + 1.0);
        }
        h.norm()
    }
}

const INF: f64 = 1.0e30;

fn warp(f: f64, fs: f64) -> f64 {
    2.0 * fs * (PI * f.min(0.49 * fs) / fs).tan()
}

fn to_z(s: C64, fs: f64) -> C64 {
    if s.norm() < 1.0e-12 {
        return C64::new(1.0, 0.0);
    }
    if s.norm() >= INF * 0.5 {
        return C64::new(-1.0, 0.0);
    }
    (C64::new(2.0 * fs, 0.0) + s) / (C64::new(2.0 * fs, 0.0) - s)
}

#[derive(Clone, Copy)]
struct List<T: Copy + Default, const N: usize> {
    v: [T; N],
    n: usize,
}

impl<T: Copy + Default, const N: usize> List<T, N> {
    fn new() -> Self {
        List { v: [T::default(); N], n: 0 }
    }
    fn push(&mut self, x: T) {
        if self.n < N {
            self.v[self.n] = x;
            self.n += 1;
        }
    }
    fn as_slice(&self) -> &[T] {
        &self.v[..self.n]
    }
    fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.v[..self.n]
    }
}

type Roots = List<C64, 40>;

#[derive(Debug, Clone, Copy, Default)]
struct Factor {
    c1: f64,
    c2: f64,
    key: f64,
}

fn factorise(roots: &Roots, fs: f64) -> List<Factor, 40> {
    let mut out = List::<Factor, 40>::new();
    let mut used = [false; 40];
    let mut reals = List::<(f64, f64), 40>::new();
    let r = roots.as_slice();
    for i in 0..r.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let s = r[i];
        let key = s.norm();
        let z = to_z(s, fs);
        if s.im.abs() <= 1.0e-9 * key.max(1.0) {
            reals.push((z.re, key));
            continue;
        }
        let mut best = r.len();
        let mut bd = 1.0e300;
        for (j, rj) in r.iter().enumerate().skip(i + 1) {
            if used[j] {
                continue;
            }
            let d = (*rj - s.conj()).norm();
            if d < bd {
                bd = d;
                best = j;
            }
        }
        if best < r.len() {
            used[best] = true;
        }
        out.push(Factor { c1: -2.0 * z.re, c2: z.norm_sqr(), key });
    }
    reals.as_mut_slice().sort_by(|a, b| a.1.total_cmp(&b.1));
    let rs = reals.as_slice();
    let mut i = 0;
    while i < rs.len() {
        if i + 1 < rs.len() {
            out.push(Factor { c1: -(rs[i].0 + rs[i + 1].0), c2: rs[i].0 * rs[i + 1].0, key: 0.5 * (rs[i].1 + rs[i + 1].1) });
        } else {
            out.push(Factor { c1: -rs[i].0, c2: 0.0, key: rs[i].1 });
        }
        i += 2;
    }
    out.as_mut_slice().sort_by(|a, b| a.key.total_cmp(&b.key));
    out
}

fn build_cascade(zeros: &Roots, poles: &Roots, target: f64, norm_f: f64, fs: f64, out: &mut Cascade) {
    let fz = factorise(zeros, fs);
    let fp = factorise(poles, fs);
    let n = MAX_SEC.min(fz.n.max(fp.n));
    out.n = n;
    for i in 0..n {
        let z = fz.as_slice().get(i).copied().unwrap_or_default();
        let p = fp.as_slice().get(i).copied().unwrap_or_default();
        out.s[i] = Bq { b0: 1.0, b1: z.c1, b2: z.c2, a1: p.c1, a2: p.c2 };
    }
    let now = if norm_f > 0.0 { out.mag_at(norm_f, fs) } else { out.mag_at(0.0, fs) };
    if now > 1.0e-30 && n > 0 {
        let k = target / now;
        out.s[0].b0 *= k;
        out.s[0].b1 *= k;
        out.s[0].b2 *= k;
    }
}

fn bw_poles(n: usize, w: f64, out: &mut Roots) {
    for m in 0..n {
        let th = PI * (2.0 * m as f64 + n as f64 + 1.0) / (2.0 * n as f64);
        out.push(C64::new(th.cos(), th.sin()) * w);
    }
}

fn aberth(c: &[f64], z: &mut [C64]) {
    let d = c.len() - 1;
    for _ in 0..200 {
        let mut step = 0.0f64;
        for i in 0..d {
            let mut p = C64::new(c[d], 0.0);
            let mut dp = C64::new(0.0, 0.0);
            for k in (0..d).rev() {
                dp = dp * z[i] + p;
                p = p * z[i] + c[k];
            }
            if dp.norm() == 0.0 {
                continue;
            }
            let ratio = p / dp;
            let mut sum = C64::new(0.0, 0.0);
            for j in 0..d {
                if j != i {
                    sum += C64::new(1.0, 0.0) / (z[i] - z[j]);
                }
            }
            let w = ratio / (C64::new(1.0, 0.0) - ratio * sum);
            z[i] -= w;
            step = step.max(w.norm() / z[i].norm().max(1.0e-300));
        }
        if step < 1.0e-13 {
            break;
        }
    }
}

fn design_dynamic(sh: &Shape, g: f64, fs: f64, out: &mut Cascade) {
    let mut zs = Roots::new();
    let mut ps = Roots::new();
    let (w1, w2) = (warp(sh.lo, fs), warp(sh.hi, fs));
    let (k1, k2) = (sh.k_lo as usize, sh.k_hi as usize);
    let g = g.clamp(1.0e-4, 1.0e4);
    if sh.has_lo {
        bw_poles(k1 / 2, w1, &mut ps);
        bw_poles(k1 / 2, w1, &mut ps);
    }
    if sh.has_hi {
        bw_poles(k2 / 2, w2, &mut ps);
        bw_poles(k2 / 2, w2, &mut ps);
    }
    let mut dc = 1.0;
    if sh.has_lo && !sh.has_hi {
        let wz = w1 * g.powf(-1.0 / k1 as f64);
        bw_poles(k1 / 2, wz, &mut zs);
        bw_poles(k1 / 2, wz, &mut zs);
    } else if sh.has_hi && !sh.has_lo {
        let wz = w2 * g.powf(1.0 / k2 as f64);
        bw_poles(k2 / 2, wz, &mut zs);
        bw_poles(k2 / 2, wz, &mut zs);
        dc = g;
    } else if sh.has_lo && sh.has_hi {
        let (a, b) = (k1 / 2, k2 / 2);
        let rb = ((w1 / w2) * (w1 / w2)).powf(b as f64);
        let mut ys = List::<C64, 40>::new();
        if a == b {
            let c = rb;
            let sq = C64::new((g + c) * (g + c) - 4.0 * c, 0.0).sqrt();
            let q = (C64::new(g + c, 0.0) + sq) * -0.5;
            for v in [q / c, C64::new(1.0, 0.0) / q] {
                for m in 0..a {
                    ys.push(C64::from_polar(v.norm().powf(1.0 / a as f64), (v.arg() + 2.0 * PI * m as f64) / a as f64));
                }
            }
        } else {
            let mut c = [0.0f64; 20];
            let deg = a + b;
            c[0] = 1.0;
            c[a] += g;
            c[b] += rb;
            c[a + b] += rb;
            for m in 0..a {
                ys.push(C64::from_polar(g.powf(-1.0 / a as f64), PI * (2.0 * m as f64 + 1.0) / a as f64 + 0.01 * (m + 1) as f64));
            }
            for m in 0..b {
                ys.push(C64::from_polar((g / rb).powf(1.0 / b as f64), PI * (2.0 * m as f64 + 1.0) / b as f64 - 0.013 * (m + 1) as f64));
            }
            aberth(&c[..=deg.min(19)], ys.as_mut_slice());
        }
        for y in ys.as_slice() {
            let mut s = C64::new(0.0, 1.0) * w1 * y.sqrt();
            if s.re > 0.0 {
                s = -s;
            }
            zs.push(s);
            zs.push(s);
        }
    } else {
        out.n = 1;
        out.s[0] = Bq { b0: g, ..Bq::default() };
        return;
    }
    build_cascade(&zs, &ps, dc, -1.0, fs, out);
}

fn design_band_pass(sh: &Shape, fs: f64, out: &mut Cascade) {
    let mut zs = Roots::new();
    let mut ps = Roots::new();
    let (k1, k2) = (sh.k_lo as usize, sh.k_hi as usize);
    if sh.has_lo {
        bw_poles(k1 / 2, warp(sh.lo, fs), &mut ps);
        bw_poles(k1 / 2, warp(sh.lo, fs), &mut ps);
        for _ in 0..k1 {
            zs.push(C64::new(0.0, 0.0));
        }
    }
    if sh.has_hi {
        bw_poles(k2 / 2, warp(sh.hi, fs), &mut ps);
        bw_poles(k2 / 2, warp(sh.hi, fs), &mut ps);
        for _ in 0..k2 {
            zs.push(C64::new(INF, 0.0));
        }
    }
    if ps.n == 0 {
        out.n = 1;
        out.s[0] = Bq::default();
        return;
    }
    let fc = geo_centre(sh).min(0.45 * fs);
    let mut sw = *sh;
    sw.lo = warp(sh.lo, fs) / (2.0 * PI);
    sw.hi = warp(sh.hi, fs) / (2.0 * PI);
    build_cascade(&zs, &ps, band_mag(&sw, warp(fc, fs) / (2.0 * PI)), fc, fs, out);
}

fn knee_curve(x: f64, w: f64) -> f64 {
    if w <= 1.0e-6 {
        return x.max(0.0);
    }
    if x <= -0.5 * w {
        return 0.0;
    }
    if x >= 0.5 * w {
        return x;
    }
    let t = x + 0.5 * w;
    t * t / (2.0 * w)
}

fn soft_min(a: f64, r: f64, w: f64) -> f64 {
    if a <= r - 0.5 * w {
        return a;
    }
    if a >= r + 0.5 * w {
        return r;
    }
    let t = a - (r - 0.5 * w);
    a - t * t / (2.0 * w)
}

pub fn ratio_law(lvl: f64, thr: f64, ratio: f64, knee_db: f64, range: f64, expand: bool) -> f64 {
    if range.abs() < 1.0e-6 {
        return 0.0;
    }
    let w = knee_db.min(30.0);
    let x = lvl - thr;
    let r = range.abs();
    let above = if expand { range > 0.0 } else { range < 0.0 };
    let m = if above {
        soft_min((if ratio >= 99.9 { 1.0 } else { 1.0 - 1.0 / ratio }) * knee_curve(x, w), r, 12.0 + 0.6 * w)
    } else {
        r.min(r * (ratio - 1.0) / 36.0 * knee_curve(-x, w))
    };
    let m = m.max(0.0);
    if range < 0.0 { -m } else { m }
}

fn linmb_f(knee: f64, range: f64, u: f64) -> f64 {
    let knee = knee.clamp(0.0, linmb::KNEE[linmb::KNEES - 1]);
    let mut ki = 0;
    while ki < linmb::KNEES - 2 && knee > linmb::KNEE[ki + 1] {
        ki += 1;
    }
    let kt = (knee - linmb::KNEE[ki]) / (linmb::KNEE[ki + 1] - linmb::KNEE[ki]);
    let ar = range.abs();
    let kr = linmb::RANGE;
    let (ri, rt) = if range < 0.0 {
        if ar >= 24.0 {
            (0, 0.0)
        } else if ar <= 3.0 {
            (2, 1.0)
        } else {
            let ri = if ar > 12.0 {
                0
            } else if ar > 6.0 {
                1
            } else {
                2
            };
            (ri, (kr[ri].abs() / ar).ln() / (kr[ri].abs() / kr[ri + 1].abs()).ln())
        }
    } else if ar <= 3.0 {
        (4, 0.0)
    } else if ar >= 18.0 {
        (6, 1.0)
    } else {
        let ri = if ar < 6.0 {
            4
        } else if ar < 12.0 {
            5
        } else {
            6
        };
        (ri, (ar / kr[ri]).ln() / (kr[ri + 1] / kr[ri]).ln())
    };
    let at = |k: usize, r: usize| -> f64 {
        let t = &linmb::F[k][r];
        let p = (u - linmb::U0) / linmb::U_STEP;
        if p <= 0.0 {
            return t[0] as f64 * 10f64.powf((u - linmb::U0) / 20.0);
        }
        if p >= (linmb::U - 1) as f64 {
            let last = t[linmb::U - 1] as f64;
            return 1.0 - (1.0 - last) * 10f64.powf(-(u - (linmb::U0 + (linmb::U - 1) as f64 * linmb::U_STEP)) / 20.0);
        }
        let i = p as usize;
        let f = p - i as f64;
        t[i] as f64 * (1.0 - f) + t[i + 1] as f64 * f
    };
    let at_k = |r: usize| at(ki, r) * (1.0 - kt) + at(ki + 1, r) * kt;
    at_k(ri) * (1.0 - rt) + at_k(ri + 1) * rt
}

pub fn range_law(lvl: f64, thr: f64, range: f64, knee: f64) -> f64 {
    if range.abs() < 1.0e-6 { 0.0 } else { range * linmb_f(knee, range, lvl - thr) }
}

fn ratio_attack_ms(pct: f64) -> f64 {
    let p = (pct * 0.01).clamp(0.0, 1.0);
    255.6 * p * p * p * p
}

fn ratio_release_ms(pct: f64) -> (f64, f64) {
    const P: [f64; 7] = [0.0, 0.1, 0.2, 0.4, 0.6, 0.8, 1.0];
    const TA: [f64; 7] = [109.1, 109.1, 119.4, 149.4, 287.6, 690.2, 1989.2];
    const TB: [f64; 7] = [2.6, 9.6, 38.6, 119.7, 176.6, 281.8, 750.9];
    let p = (pct * 0.01).clamp(0.0, 1.0);
    let mut i = 0;
    while i < 5 && p > P[i + 1] {
        i += 1;
    }
    let t = (p - P[i]) / (P[i + 1] - P[i]);
    ((TA[i].ln() * (1.0 - t) + TA[i + 1].ln() * t).exp(), (TB[i].ln() * (1.0 - t) + TB[i + 1].ln() * t).exp())
}

fn coef_for(ms: f64, fs: f64) -> f64 {
    if ms <= 1.0e-3 { 1.0 } else { 1.0 - (-1000.0 / (ms * fs)).exp() }
}

const RT_ATTACK: f64 = 2.0;
const RT_CAP: f64 = 5.0;
const RT_TAIL: f64 = 0.002;
const RT_ARC: f64 = 3.0;

#[derive(Debug, Clone, Copy, Default)]
struct Envelope {
    s1: f64,
    s2: f64,
    arc_mem: f64,
}

impl Envelope {
    fn ratio_step(&mut self, target: f64, c_att: f64, c_rel_a: f64, c_rel_b: f64) -> f64 {
        self.s1 += (target - self.s1) * if target > self.s1 { c_att } else { c_rel_a };
        if self.s1 > self.s2 {
            self.s2 = self.s1;
        } else {
            self.s2 += (self.s1 - self.s2) * c_rel_b;
        }
        self.s2
    }

    fn range_step(&mut self, target: f64, att_ms: f64, rel_ms: f64, opto: bool, arc: bool, fs: f64) -> f64 {
        let rel_s = rel_ms * 0.001;
        let m = self.s1;
        let m = if target > m {
            m + (target - m) * coef_for(RT_ATTACK * att_ms, fs)
        } else {
            let d = m - target;
            let mut speed = 1.0;
            if arc {
                speed += RT_ARC * (1.0 - self.arc_mem / m.max(1.0e-3)).clamp(0.0, 1.0);
            }
            let rate = if !opto { (d / RT_TAIL).min(RT_CAP / rel_s) } else { d / (rel_s * (0.12 + 0.88 * (-d / 0.35).exp())).max(0.002) };
            target.max(m - rate * speed / fs)
        };
        self.s1 = m;
        self.arc_mem += (m - self.arc_mem) * coef_for(2.0 * rel_ms, fs);
        self.s2 = m;
        m
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandSettings {
    pub active: bool,
    pub solo: bool,
    pub sc: bool,
    pub expand: bool,
    pub place: usize,
    pub lo: f32,
    pub hi: f32,
    pub slope_lo: f32,
    pub slope_hi: f32,
    pub thr: f32,
    pub range: f32,
    pub ratio: f32,
    pub knee: f32,
    pub atk: f32,
    pub rel: f32,
    pub la: f32,
    pub level: f32,
    pub latk: f32,
    pub lrel: f32,
}

impl Default for BandSettings {
    fn default() -> Self {
        BandSettings {
            active: false,
            solo: false,
            sc: false,
            expand: false,
            place: STEREO,
            lo: 100.0,
            hi: 1000.0,
            slope_lo: 24.0,
            slope_hi: 24.0,
            thr: -18.0,
            range: -6.0,
            ratio: 4.0,
            knee: 24.0,
            atk: 20.0,
            rel: 20.0,
            la: 1.0,
            level: 0.0,
            latk: 16.0,
            lrel: 25.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GlobalSettings {
    pub law: usize,
    pub lknee: f32,
    pub arc: bool,
    pub opto: bool,
    pub mix: f32,
}

pub fn shape_of(b: &BandSettings, fs: f64) -> Shape {
    Shape {
        lo: b.lo as f64,
        hi: b.hi as f64,
        k_lo: b.slope_lo as f64 / 6.0,
        k_hi: b.slope_hi as f64 / 6.0,
        has_lo: b.lo > OPEN_LO,
        has_hi: b.hi < OPEN_HI && (b.hi as f64) < 0.45 * fs,
    }
}

const CHUNK: usize = 32;

struct BandRt {
    shape: Shape,
    even: Shape,
    shape_valid: bool,
    det: Cascade,
    dyn_: Cascade,
    det_st: [[BqSt; MAX_SEC]; 2],
    dyn_st: [[BqSt; MAX_SEC]; 2],
    lis_st: [[BqSt; MAX_SEC]; 2],
    pk: f64,
    pk_decay: f64,
    hold_n: usize,
    held: usize,
    since_pk: usize,
    held1: usize,
    pk1: f64,
    env: Envelope,
    gc: Vec<f32>,
    act: f64,
    last_gdb: f64,
    law_was: usize,
    ker_cur: Vec<Complex<f32>>,
    ker_old: Vec<Complex<f32>>,
    out: [Vec<f32>; 2],
    ker_shape: Shape,
    ker_valid: bool,
    fade: bool,
}

impl BandRt {
    fn new(ring: usize, m: usize, h: usize) -> BandRt {
        let z = Complex::new(0.0, 0.0);
        BandRt {
            shape: Shape::default(),
            even: Shape::default(),
            shape_valid: false,
            det: Cascade::default(),
            dyn_: Cascade::default(),
            det_st: [[BqSt::default(); MAX_SEC]; 2],
            dyn_st: [[BqSt::default(); MAX_SEC]; 2],
            lis_st: [[BqSt::default(); MAX_SEC]; 2],
            pk: 0.0,
            pk_decay: 0.999,
            hold_n: 48,
            held: 0,
            since_pk: 0,
            held1: 0,
            pk1: 0.0,
            env: Envelope::default(),
            gc: vec![0.0; ring],
            act: 0.0,
            last_gdb: 1.0e9,
            law_was: usize::MAX,
            ker_cur: vec![z; m],
            ker_old: vec![z; m],
            out: [vec![0.0; h], vec![0.0; h]],
            ker_shape: Shape::default(),
            ker_valid: false,
            fade: false,
        }
    }

    fn clear_states(&mut self) {
        self.det_st = [[BqSt::default(); MAX_SEC]; 2];
        self.dyn_st = [[BqSt::default(); MAX_SEC]; 2];
        self.lis_st = [[BqSt::default(); MAX_SEC]; 2];
    }
}

fn chan_count(place: usize) -> usize {
    if place == STEREO { 2 } else { 1 }
}

fn domain_of(place: usize, j: usize) -> usize {
    match place {
        STEREO => j,
        LEFT => 0,
        RIGHT => 1,
        MID => 2,
        _ => 3,
    }
}

pub struct MbEngine {
    fs: f64,
    la_max: usize,
    n: usize,
    h: usize,
    m: usize,
    lfir: usize,
    min_gap: usize,
    hold_min: usize,
    hold_max: usize,
    dec1: f64,
    ring_mask: usize,
    pos: usize,
    hop_pos: usize,
    lin_mode: bool,
    la_on: bool,
    rt: Vec<BandRt>,
    la_buf: [Vec<f32>; 2],
    fir_dry: [Vec<f32>; 2],
    dry_buf: [Vec<f32>; 2],
    chunk: [[f64; CHUNK]; 2],
    fft_n: Arc<dyn Fft<f32>>,
    fft_m: Arc<dyn Fft<f32>>,
    ifft_m: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex<f32>>,
    hist: [Vec<f32>; 4],
    x: [Vec<Complex<f32>>; 4],
    work: Vec<Complex<f32>>,
    work2: Vec<Complex<f32>>,
    win: Vec<f32>,
    cbuf: Vec<Complex<f32>>,
    pub gc_now: [f32; BANDS],
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

impl MbEngine {
    pub fn new(fs: f64) -> MbEngine {
        let la_max = (0.02 * fs).round() as usize;
        let mut mult = 1;
        while fs > 48_000.0 * mult as f64 * 1.5 {
            mult *= 2;
        }
        let (n, h) = (4096 * mult, 1024 * mult);
        let m = 2 * n;
        let lfir = n / 2 + h;
        let mut ring = 1;
        while ring < lfir + la_max + 64 {
            ring <<= 1;
        }
        let hold_min = (0.001 * fs).round() as usize;
        let mut planner = FftPlanner::<f32>::new();
        let fft_n = planner.plan_fft_inverse(n);
        let fft_m = planner.plan_fft_forward(m);
        let ifft_m = planner.plan_fft_inverse(m);
        let sl = fft_n.get_inplace_scratch_len().max(fft_m.get_inplace_scratch_len()).max(ifft_m.get_inplace_scratch_len());
        let beta = 8.0;
        let i0b = bessel_i0(beta);
        let win = (0..n)
            .map(|j| {
                let t = 2.0 * j as f64 / n as f64 - 1.0;
                (bessel_i0(beta * (1.0 - t * t).max(0.0).sqrt()) / i0b) as f32
            })
            .collect();
        let z = Complex::new(0.0, 0.0);
        let rv = || vec![0.0f32; ring];
        MbEngine {
            fs,
            la_max,
            n,
            h,
            m,
            lfir,
            min_gap: (0.0002 * fs).round() as usize,
            hold_min,
            hold_max: (0.05 * fs).round() as usize,
            dec1: (-1.0 / hold_min.max(1) as f64).exp(),
            ring_mask: ring - 1,
            pos: 0,
            hop_pos: 0,
            lin_mode: false,
            la_on: true,
            rt: (0..BANDS).map(|_| BandRt::new(ring, m, h)).collect(),
            la_buf: [rv(), rv()],
            fir_dry: [rv(), rv()],
            dry_buf: [rv(), rv()],
            chunk: [[0.0; CHUNK]; 2],
            fft_n,
            fft_m,
            ifft_m,
            scratch: vec![z; sl],
            hist: [vec![0.0; m], vec![0.0; m], vec![0.0; m], vec![0.0; m]],
            x: [vec![z; m], vec![z; m], vec![z; m], vec![z; m]],
            work: vec![z; m],
            work2: vec![z; m],
            win,
            cbuf: vec![z; n],
            gc_now: [0.0; BANDS],
        }
    }

    pub fn latency_for(&self, lin: bool, la: bool) -> usize {
        (if la { self.la_max } else { 0 }) + if lin { self.lfir } else { 0 }
    }

    pub fn latency(&self) -> usize {
        self.latency_for(self.lin_mode, self.la_on)
    }

    pub fn set_mode(&mut self, lin: bool, la: bool) {
        if lin != self.lin_mode || la != self.la_on {
            self.lin_mode = lin;
            self.la_on = la;
            self.reset();
        }
    }

    pub fn reset(&mut self) {
        for b in self.rt.iter_mut() {
            b.gc.iter_mut().for_each(|v| *v = 0.0);
            for o in b.out.iter_mut() {
                o.iter_mut().for_each(|v| *v = 0.0);
            }
            b.env = Envelope::default();
            b.pk = 0.0;
            b.act = 0.0;
            b.last_gdb = 1.0e9;
            b.law_was = usize::MAX;
            b.ker_valid = false;
            b.fade = false;
            b.shape_valid = false;
            b.clear_states();
        }
        for c in 0..2 {
            for v in [&mut self.la_buf[c], &mut self.fir_dry[c], &mut self.dry_buf[c]] {
                v.iter_mut().for_each(|x| *x = 0.0);
            }
        }
        for h in self.hist.iter_mut() {
            h.iter_mut().for_each(|x| *x = 0.0);
        }
        self.pos = 0;
        self.hop_pos = 0;
        self.gc_now = [0.0; BANDS];
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], sc: Option<(&[f32], &[f32])>, bs: &[BandSettings; BANDS], gs: &GlobalSettings) {
        let mut any_solo = false;
        for b in 0..BANDS {
            self.update_shape(b, &bs[b]);
            any_solo |= bs[b].active && bs[b].solo;
        }
        let lat = self.latency();
        let n = l.len().min(r.len());
        let mask = self.ring_mask;
        let mut i0 = 0;
        while i0 < n {
            let m = CHUNK.min(n - i0);
            for i in 0..m {
                let t = (self.pos + i) & mask;
                let (xl, xr) = (l[i0 + i] as f64, r[i0 + i] as f64);
                self.dry_buf[0][t] = xl as f32;
                self.dry_buf[1][t] = xr as f32;
                let dom = [xl, xr, 0.5 * (xl + xr), 0.5 * (xl - xr)];
                let sdom = match sc {
                    Some((a, c)) => {
                        let (a, c) = (a[i0 + i] as f64, c[i0 + i] as f64);
                        [a, c, 0.5 * (a + c), 0.5 * (a - c)]
                    }
                    None => [0.0; 4],
                };
                for b in 0..BANDS {
                    let d = if bs[b].sc && sc.is_some() { &sdom } else { &dom };
                    self.detect(b, &bs[b], gs, d, t);
                }
                self.la_buf[0][t] = xl as f32;
                self.la_buf[1][t] = xr as f32;
                let td = (t.wrapping_sub(if self.la_on { self.la_max } else { 0 })) & mask;
                self.chunk[0][i] = self.la_buf[0][td] as f64;
                self.chunk[1][i] = self.la_buf[1][td] as f64;
            }
            if self.lin_mode {
                self.linear_chunk(m, bs, gs, any_solo);
            } else {
                self.dynamic_chunk(m, bs, gs, any_solo);
            }
            for i in 0..m {
                let td = (self.pos + i).wrapping_sub(lat) & mask;
                let (dl, dr) = (self.dry_buf[0][td], self.dry_buf[1][td]);
                l[i0 + i] = dl + gs.mix * (self.chunk[0][i] as f32 - dl);
                r[i0 + i] = dr + gs.mix * (self.chunk[1][i] as f32 - dr);
            }
            self.pos = (self.pos + m) & mask;
            i0 += m;
        }
    }

    fn update_shape(&mut self, b: usize, s: &BandSettings) {
        let fs = self.fs;
        let r = &mut self.rt[b];
        let sh = shape_of(s, fs);
        if r.shape_valid && sh == r.shape {
            return;
        }
        let was = r.shape_valid;
        r.shape = sh;
        r.even = even_shape(sh);
        r.shape_valid = true;
        r.last_gdb = 1.0e9;
        let old_n = r.det.n;
        design_band_pass(&r.even, fs, &mut r.det);
        if !was || old_n != r.det.n {
            r.det_st = [[BqSt::default(); MAX_SEC]; 2];
            r.lis_st = [[BqSt::default(); MAX_SEC]; 2];
        }
    }

    fn detect(&mut self, b: usize, s: &BandSettings, gs: &GlobalSettings, dom: &[f64; 4], t: usize) {
        let fs = self.fs;
        let (min_gap, hold_min, hold_max, dec1) = (self.min_gap, self.hold_min, self.hold_max, self.dec1);
        let r = &mut self.rt[b];
        if !s.active {
            r.gc[t] = 0.0;
            r.env = Envelope::default();
            r.pk = 0.0;
            r.law_was = usize::MAX;
            return;
        }
        let mut peak = 0.0f64;
        for j in 0..chan_count(s.place) {
            peak = peak.max(r.det.tick(&mut r.det_st[j], dom[domain_of(s.place, j)]).abs());
        }
        r.since_pk += 1;
        if r.since_pk > min_gap && peak >= 0.7 * r.pk {
            r.hold_n = ((1.5 * r.since_pk as f64) as usize).clamp(hold_min, hold_max);
            r.pk_decay = (-1.0 / r.hold_n.max(1) as f64).exp();
            r.since_pk = 0;
        }
        if peak >= r.pk {
            r.pk = peak;
            r.held = r.hold_n;
        } else if r.held > 0 {
            r.held -= 1;
        } else {
            r.pk = peak.max(r.pk * r.pk_decay);
        }
        let lvl = 20.0 * (r.pk + 1.0e-9).log10();
        if r.law_was != gs.law {
            r.env = Envelope::default();
        }
        let m = if gs.law == LAW_RATIO {
            let target = ratio_law(lvl, s.thr as f64, s.ratio as f64, s.knee as f64, s.range as f64, s.expand);
            let (ta, tb) = ratio_release_ms(s.rel as f64);
            r.env.ratio_step(target.abs(), coef_for(ratio_attack_ms(s.atk as f64), fs), coef_for(ta, fs), coef_for(tb, fs))
        } else {
            if peak >= r.pk1 {
                r.pk1 = peak;
                r.held1 = hold_min;
            } else if r.held1 > 0 {
                r.held1 -= 1;
            } else {
                r.pk1 = peak.max(r.pk1 * dec1);
            }
            let target = range_law(20.0 * (r.pk1 + 1.0e-9).log10(), s.thr as f64, s.range as f64, gs.lknee as f64);
            r.env.range_step(target.abs(), s.latk as f64, s.lrel as f64, gs.opto, gs.arc, fs)
        };
        r.law_was = gs.law;
        r.gc[t] = (if s.range < 0.0 { -m } else { m }) as f32;
    }

    fn gc_delay(&self, s: &BandSettings, gs: &GlobalSettings) -> usize {
        let mut d = if self.lin_mode { self.lfir as i64 } else { 0 };
        if self.la_on {
            let la = if gs.law == LAW_RATIO { (s.la as f64).clamp(0.0, 20.0) } else { 0.0 };
            d += self.la_max as i64 - (la * 0.001 * self.fs).round() as i64;
        }
        d.max(0) as usize
    }

    fn step_act(fs: f64, r: &mut BandRt, on: bool, m: usize) -> f64 {
        let step = m as f64 / (0.02 * fs);
        r.act = if on { (r.act + step).min(1.0) } else { (r.act - step).max(0.0) };
        r.act
    }

    fn dynamic_chunk(&mut self, m: usize, bs: &[BandSettings; BANDS], gs: &GlobalSettings, any_solo: bool) {
        let fs = self.fs;
        let input = self.chunk;
        let mut sig = [[0.0f64; CHUNK]; 4];
        let mut solo_out = [[0.0f64; CHUNK]; 4];
        for stage in 0..2 {
            if stage == 1 {
                for i in 0..m {
                    sig[2][i] = 0.5 * (self.chunk[0][i] + self.chunk[1][i]);
                    sig[3][i] = 0.5 * (self.chunk[0][i] - self.chunk[1][i]);
                }
            }
            for b in 0..BANDS {
                let s = &bs[b];
                let ms = s.place == MID || s.place == SIDE;
                if ms != (stage == 1) {
                    continue;
                }
                let delay = self.gc_delay(s, gs);
                let t_apply = self.pos.wrapping_sub(delay) & self.ring_mask;
                let r = &mut self.rt[b];
                let act = Self::step_act(fs, r, s.active, m);
                if act <= 0.0 {
                    r.last_gdb = 1.0e9;
                    self.gc_now[b] = 0.0;
                    continue;
                }
                let gcv = r.gc[t_apply];
                self.gc_now[b] = gcv;
                let gdb = act * (s.level as f64 + gcv as f64);
                if (gdb - r.last_gdb).abs() > 0.005 {
                    let before = r.dyn_.n;
                    design_dynamic(&r.even, 10f64.powf(gdb / 20.0), fs, &mut r.dyn_);
                    if before != r.dyn_.n {
                        r.dyn_st = [[BqSt::default(); MAX_SEC]; 2];
                    }
                    r.last_gdb = gdb;
                }
                let g_lin = 10f64.powf(gdb / 20.0);
                for j in 0..chan_count(s.place) {
                    let d = domain_of(s.place, j);
                    if any_solo && s.solo {
                        for i in 0..m {
                            let src = match d {
                                0 | 1 => input[d][i],
                                2 => 0.5 * (input[0][i] + input[1][i]),
                                _ => 0.5 * (input[0][i] - input[1][i]),
                            };
                            solo_out[d][i] += g_lin * r.det.tick(&mut r.lis_st[j], src);
                        }
                    }
                    let x: &mut [f64; CHUNK] = if d < 2 { &mut self.chunk[d] } else { &mut sig[d] };
                    for v in x.iter_mut().take(m) {
                        *v = r.dyn_.tick(&mut r.dyn_st[j], *v);
                    }
                }
            }
            if stage == 1 {
                for i in 0..m {
                    self.chunk[0][i] = sig[2][i] + sig[3][i];
                    self.chunk[1][i] = sig[2][i] - sig[3][i];
                }
            }
        }
        if any_solo {
            for i in 0..m {
                self.chunk[0][i] = solo_out[0][i] + solo_out[2][i] + solo_out[3][i];
                self.chunk[1][i] = solo_out[1][i] + solo_out[2][i] - solo_out[3][i];
            }
        }
    }

    fn make_kernel(&mut self, b: usize) {
        let (n, fs) = (self.n, self.fs);
        let sh = self.rt[b].shape;
        for k in 0..=n / 2 {
            let v = Complex::new(band_mag(&sh, (k as f64 * fs / n as f64).max(1.0e-3)) as f32, 0.0);
            self.cbuf[k] = v;
            if k > 0 && k < n / 2 {
                self.cbuf[n - k] = v;
            }
        }
        self.fft_n.process_with_scratch(&mut self.cbuf, &mut self.scratch);
        let scale = 1.0 / n as f32;
        let ker = &mut self.rt[b].ker_cur;
        ker.iter_mut().for_each(|c| *c = Complex::new(0.0, 0.0));
        for j in 0..n {
            ker[j] = Complex::new(self.cbuf[(j + n / 2) % n].re * scale * self.win[j], 0.0);
        }
        self.fft_m.process_with_scratch(ker, &mut self.scratch);
    }

    fn hop(&mut self, bs: &[BandSettings; BANDS]) {
        let (m, h) = (self.m, self.h);
        let mut need = [false; 4];
        for b in 0..BANDS {
            if bs[b].active || self.rt[b].act > 0.0 {
                for j in 0..chan_count(bs[b].place) {
                    need[domain_of(bs[b].place, j)] = true;
                }
            }
        }
        for d in 0..4 {
            if !need[d] {
                continue;
            }
            for (o, &v) in self.x[d].iter_mut().zip(self.hist[d].iter()) {
                *o = Complex::new(v, 0.0);
            }
            self.fft_m.process_with_scratch(&mut self.x[d], &mut self.scratch);
        }
        let scale = 1.0 / m as f32;
        for b in 0..BANDS {
            let s = bs[b];
            if !(s.active || self.rt[b].act > 0.0) {
                let r = &mut self.rt[b];
                for o in r.out.iter_mut() {
                    o.iter_mut().for_each(|v| *v = 0.0);
                }
                r.ker_valid = false;
                continue;
            }
            self.rt[b].fade = false;
            if !self.rt[b].ker_valid || self.rt[b].ker_shape != self.rt[b].shape {
                if self.rt[b].ker_valid {
                    let r = &mut self.rt[b];
                    std::mem::swap(&mut r.ker_old, &mut r.ker_cur);
                    r.fade = true;
                }
                self.make_kernel(b);
                let r = &mut self.rt[b];
                r.ker_shape = r.shape;
                r.ker_valid = true;
            }
            for j in 0..chan_count(s.place) {
                let xd = &self.x[domain_of(s.place, j)];
                let r = &mut self.rt[b];
                for ((o, a), k) in self.work.iter_mut().zip(xd.iter()).zip(r.ker_cur.iter()) {
                    *o = a * k;
                }
                self.ifft_m.process_with_scratch(&mut self.work, &mut self.scratch);
                if r.fade {
                    for ((o, a), k) in self.work2.iter_mut().zip(xd.iter()).zip(r.ker_old.iter()) {
                        *o = a * k;
                    }
                    self.ifft_m.process_with_scratch(&mut self.work2, &mut self.scratch);
                    for i in 0..h {
                        let w = 0.5 - 0.5 * (std::f32::consts::PI * (i as f32 + 0.5) / h as f32).cos();
                        r.out[j][i] = scale * (w * self.work[m - h + i].re + (1.0 - w) * self.work2[m - h + i].re);
                    }
                } else {
                    for i in 0..h {
                        r.out[j][i] = scale * self.work[m - h + i].re;
                    }
                }
            }
        }
    }

    fn linear_chunk(&mut self, m: usize, bs: &[BandSettings; BANDS], gs: &GlobalSettings, any_solo: bool) {
        let fs = self.fs;
        let (mm, h, mask) = (self.m, self.h, self.ring_mask);
        for i in 0..m {
            let (xl, xr) = (self.chunk[0][i], self.chunk[1][i]);
            let dom = [xl, xr, 0.5 * (xl + xr), 0.5 * (xl - xr)];
            let t = (self.pos + i) & mask;
            self.fir_dry[0][t] = xl as f32;
            self.fir_dry[1][t] = xr as f32;
            let tf = t.wrapping_sub(self.lfir) & mask;
            let mut yl = if any_solo { 0.0 } else { self.fir_dry[0][tf] as f64 };
            let mut yr = if any_solo { 0.0 } else { self.fir_dry[1][tf] as f64 };
            for b in 0..BANDS {
                let s = &bs[b];
                let delay = self.gc_delay(s, gs);
                let r = &mut self.rt[b];
                let act = Self::step_act(fs, r, s.active, 1);
                if act <= 0.0 {
                    self.gc_now[b] = 0.0;
                    continue;
                }
                let gcv = r.gc[t.wrapping_sub(delay) & mask];
                self.gc_now[b] = gcv;
                let g = 10f64.powf(act * (s.level as f64 + gcv as f64) / 20.0);
                let k = if any_solo {
                    if s.solo { g } else { 0.0 }
                } else {
                    g - 1.0
                };
                if k == 0.0 {
                    continue;
                }
                let v0 = k * r.out[0][self.hop_pos] as f64;
                match s.place {
                    STEREO => {
                        yl += v0;
                        yr += k * r.out[1][self.hop_pos] as f64;
                    }
                    LEFT => yl += v0,
                    RIGHT => yr += v0,
                    MID => {
                        yl += v0;
                        yr += v0;
                    }
                    _ => {
                        yl += v0;
                        yr -= v0;
                    }
                }
            }
            self.chunk[0][i] = yl;
            self.chunk[1][i] = yr;
            for d in 0..4 {
                self.hist[d][mm - h + self.hop_pos] = dom[d] as f32;
            }
            self.hop_pos += 1;
            if self.hop_pos == h {
                self.hop(bs);
                for d in 0..4 {
                    self.hist[d].copy_within(h..mm, 0);
                }
                self.hop_pos = 0;
            }
        }
    }
}

pub struct OpenMb {
    p: Params,
    sr: f32,
    ch: usize,
    eng: MbEngine,
    set: [BandSettings; BANDS],
    gain: f32,
    work: [Vec<f32>; 2],
    pre: [Vec<f32>; 2],
    key: [Vec<f32>; 2],
    scope: Arc<Scope>,
}

impl Default for OpenMb {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenMb {
    pub fn new() -> OpenMb {
        OpenMb {
            p: Params::new(&PARAMS),
            sr: 48_000.0,
            ch: 2,
            eng: MbEngine::new(48_000.0),
            set: [BandSettings::default(); BANDS],
            gain: 1.0,
            work: [Vec::new(), Vec::new()],
            pre: [Vec::new(), Vec::new()],
            key: [Vec::new(), Vec::new()],
            scope: Scope::new(),
        }
    }

    fn update(&mut self) {}

    fn read_settings(&mut self, mono: bool) -> GlobalSettings {
        for b in 0..BANDS {
            let i = b * PER_BAND;
            let v = |k: usize| self.p.v(i + k);
            let s = BandSettings {
                active: self.p.on(i + K_USED) && !self.p.on(i + K_BYP),
                solo: self.p.on(i + K_SOLO),
                sc: self.p.on(i + K_SC),
                expand: self.p.choice(i + K_MODE) == 1,
                place: self.p.choice(i + K_PLACE),
                lo: v(K_LO),
                hi: v(K_HI),
                slope_lo: v(K_SLO),
                slope_hi: v(K_SHI),
                thr: v(K_THR),
                range: v(K_RANGE),
                ratio: v(K_RATIO),
                knee: v(K_KNEE),
                atk: v(K_ATK),
                rel: v(K_REL),
                la: v(K_LA),
                level: v(K_LEVEL),
                latk: v(K_LATK),
                lrel: v(K_LREL),
            };
            let mut s = s;
            if mono && s.place != STEREO {
                if s.place == SIDE || s.place == RIGHT {
                    s.active = false;
                }
                s.place = STEREO;
            }
            self.set[b] = s;
        }
        GlobalSettings {
            law: self.p.choice(G_LAW),
            lknee: self.p.v(G_LKNEE),
            arc: self.p.choice(G_RELMODE) == 0,
            opto: self.p.choice(G_BEHAVIOR) == 1,
            mix: self.p.v(G_MIX) * 0.01,
        }
    }

    fn run(&mut self, io: &mut [Vec<f32>], key: Option<&[Vec<f32>]>, frames: usize) {
        let nch = io.len().min(self.ch).min(2);
        if nch == 0 {
            return;
        }
        let frames = frames.min(io[..nch].iter().map(Vec::len).min().unwrap_or(0));
        self.eng.set_mode(self.p.choice(G_PHASE) == 1, self.p.on(G_LOOKAHEAD));
        let gs = self.read_settings(nch == 1);
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
            self.eng.process(&mut wl[..n], &mut wr[..n], sc, &self.set, &gs);
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
                d.bands[..BANDS].copy_from_slice(&self.eng.gc_now);
            }
            off += n;
        }
    }
}

impl Plugin for OpenMb {
    param_plumbing!(OPEN_MB_INFO);

    fn prepare(&mut self, sample_rate: f32, max_block: usize, channels: usize) {
        self.sr = if sample_rate.is_finite() && sample_rate > 0.0 { sample_rate.clamp(8_000.0, 384_000.0) } else { 48_000.0 };
        self.ch = channels.clamp(1, 2);
        let n = max_block.clamp(1, 16_384);
        for v in self.work.iter_mut().chain(self.pre.iter_mut()).chain(self.key.iter_mut()) {
            *v = vec![0.0; n];
        }
        self.eng = MbEngine::new(self.sr as f64);
        self.eng.set_mode(self.p.choice(G_PHASE) == 1, self.p.on(G_LOOKAHEAD));
        self.eng.reset();
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
        self.eng.latency_for(self.p.choice(G_PHASE) == 1, self.p.on(G_LOOKAHEAD))
    }

    fn gain_reduction_db(&self) -> f32 {
        self.eng.gc_now.iter().fold(0.0f32, |a, &g| a.max(-g))
    }

    fn scope(&self) -> Option<Arc<Scope>> {
        Some(Arc::clone(&self.scope))
    }
}

pub fn band_from(get: impl Fn(&str) -> f32, b: usize) -> BandSettings {
    let i = b * PER_BAND;
    let v = |k: usize| get(PARAMS[i + k].id);
    BandSettings {
        active: v(K_USED) >= 0.5 && v(K_BYP) < 0.5,
        solo: v(K_SOLO) >= 0.5,
        sc: v(K_SC) >= 0.5,
        expand: v(K_MODE) >= 0.5,
        place: v(K_PLACE).round().max(0.0) as usize,
        lo: v(K_LO),
        hi: v(K_HI),
        slope_lo: v(K_SLO),
        slope_hi: v(K_SHI),
        thr: v(K_THR),
        range: v(K_RANGE),
        ratio: v(K_RATIO),
        knee: v(K_KNEE),
        atk: v(K_ATK),
        rel: v(K_REL),
        la: v(K_LA),
        level: v(K_LEVEL),
        latk: v(K_LATK),
        lrel: v(K_LREL),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level_after(mb: &mut OpenMb, f: f32, amp: f32) -> f32 {
        let sr = 48_000.0;
        let mut out = Vec::new();
        for blk in 0..120 {
            let x: Vec<f32> = (0..512).map(|i| amp * (std::f32::consts::TAU * f * (blk * 512 + i) as f32 / sr).sin()).collect();
            let mut io = vec![x.clone(), x];
            mb.process(&mut io, 512);
            out.extend_from_slice(&io[0]);
        }
        let tail = &out[out.len() - 12_000..];
        let rms = (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt();
        20.0 * (rms / (amp / std::f32::consts::SQRT_2)).log10()
    }

    #[test]
    fn a_loud_band_is_compressed_and_others_pass() {
        for phase in [0.0, 1.0] {
            let mut mb = OpenMb::new();
            mb.set_param("phase", phase);
            mb.set_param("b3used", 1.0);
            mb.set_param("b3thr", -30.0);
            mb.set_param("b3range", -12.0);
            mb.prepare(48_000.0, 512, 2);
            let inside = level_after(&mut mb, 550.0, 0.5);
            assert!(inside < -3.0, "phase {phase}: {inside}");
            let mut mb = OpenMb::new();
            mb.set_param("phase", phase);
            mb.set_param("b3used", 1.0);
            mb.set_param("b3thr", -30.0);
            mb.set_param("b3range", -12.0);
            mb.prepare(48_000.0, 512, 2);
            let outside = level_after(&mut mb, 8000.0, 0.5);
            assert!(outside.abs() < 0.3, "phase {phase}: {outside}");
        }
    }

    #[test]
    fn dynamic_filter_reaches_its_gain_in_band() {
        let sh = even_shape(Shape { lo: 500.0, hi: 2000.0, k_lo: 4.0, k_hi: 4.0, has_lo: true, has_hi: true });
        let mut c = Cascade::default();
        design_dynamic(&sh, 0.5, 48_000.0, &mut c);
        let at = 20.0 * c.mag_at(1000.0, 48_000.0).log10();
        let want = 20.0 * (1.0 - 0.5 * band_mag(&sh, 1000.0)).log10();
        assert!((at - want).abs() < 0.3, "{at} vs {want}");
        assert!((20.0 * c.mag_at(50.0, 48_000.0).log10()).abs() < 0.2);
    }
}
