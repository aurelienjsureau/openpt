use crate::params::{Params, choice, lin, log, param_plumbing, toggle};
use crate::scope::{Column, Scope};
use crate::{Category, ParamInfo, Plugin, PluginInfo, Unit};
use std::sync::Arc;

pub const MODES: &[&str] = &["Single Vocal", "Allround"];
pub const VOCAL: usize = 0;
pub const BANDS: &[&str] = &["Split Band", "Wideband"];
pub const SPLIT: usize = 0;

const P_MODE: usize = 0;
const P_THR: usize = 1;
const P_RANGE: usize = 2;
const P_BAND: usize = 3;
const P_LO: usize = 4;
const P_HI: usize = 5;
const P_LOOKAHEAD: usize = 6;
const P_LINK: usize = 7;
const P_LISTEN: usize = 8;

static PARAMS: [ParamInfo; 9] = [
    choice("mode", "Mode", MODES, 0),
    lin("threshold", "Threshold", -60.0, 0.0, -24.0, Unit::Db),
    lin("range", "Range", 0.0, 24.0, 12.0, Unit::Db),
    choice("band", "Processing", BANDS, 0),
    log("freq_lo", "Detection Low", 2_000.0, 16_000.0, 5_500.0, Unit::Hz),
    log("freq_hi", "Detection High", 4_000.0, 20_000.0, 16_000.0, Unit::Hz),
    lin("lookahead", "Lookahead", 0.0, 15.0, 5.0, Unit::Ms),
    lin("link", "Stereo Link", 0.0, 100.0, 100.0, Unit::Percent),
    toggle("listen", "Listen", false),
];

pub static OPEN_DS_INFO: PluginInfo = PluginInfo {
    id: "open_ds",
    name: "OpenDS",
    short_name: "OpenDS",
    category: Category::Dynamics,
    params: &PARAMS,
    is_instrument: false,
    audiosuite: true,
};

const KNEE: f32 = 3.0;
const RELEASE_MS: f32 = 60.0;
const MAX_LOOKAHEAD_MS: f32 = 15.0;

pub fn reduction(det: f32, thr: f32, range: f32) -> f32 {
    let x = det - thr;
    let r = if 2.0 * x < -KNEE {
        0.0
    } else if 2.0 * x <= KNEE {
        let t = x + 0.5 * KNEE;
        t * t / (2.0 * KNEE)
    } else {
        x
    };
    r.clamp(0.0, range.max(0.0))
}

#[derive(Debug, Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn set(&mut self, high: bool, f: f32, sr: f32) {
        let w0 = std::f32::consts::TAU * f.clamp(20.0, sr * 0.45) / sr;
        let (c, s) = (w0.cos(), w0.sin());
        let alpha = s / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let (b0, b1) = if high { ((1.0 + c) * 0.5, -(1.0 + c)) } else { ((1.0 - c) * 0.5, 1.0 - c) };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b0 / a0;
        self.a1 = -2.0 * c / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn clear(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Chan {
    det_hp: [Biquad; 2],
    det_lp: Biquad,
    rest_lp: Biquad,
    split: [Biquad; 2],
    band_env: f32,
    rest_env: f32,
    gr: f32,
}

impl Chan {
    fn clear(&mut self) {
        self.det_hp.iter_mut().chain(self.split.iter_mut()).chain([&mut self.det_lp, &mut self.rest_lp]).for_each(Biquad::clear);
        self.band_env = 0.0;
        self.rest_env = 0.0;
        self.gr = 0.0;
    }
}

pub struct OpenDs {
    p: Params,
    sr: f32,
    ch: usize,
    chans: [Chan; 2],
    delay: [Vec<f32>; 2],
    dpos: usize,
    tuned: (f32, f32),
    gr_now: f32,
    col_len: usize,
    col_n: usize,
    scope: Arc<Scope>,
}

impl Default for OpenDs {
    fn default() -> Self {
        Self::new()
    }
}

fn ms_coef(ms: f32, sr: f32) -> f32 {
    if ms <= 0.0 { 1.0 } else { 1.0 - (-1000.0 / (ms * sr)).exp() }
}

impl OpenDs {
    pub fn new() -> OpenDs {
        OpenDs {
            p: Params::new(&PARAMS),
            sr: 48_000.0,
            ch: 2,
            chans: [Chan::default(); 2],
            delay: [Vec::new(), Vec::new()],
            dpos: 0,
            tuned: (0.0, 0.0),
            gr_now: 0.0,
            col_len: 480,
            col_n: 0,
            scope: Scope::new(),
        }
    }

    fn update(&mut self) {}

    fn lookahead_samples(&self) -> usize {
        ((self.p.v(P_LOOKAHEAD).clamp(0.0, MAX_LOOKAHEAD_MS) * 0.001 * self.sr).round() as usize).min(self.delay[0].len().saturating_sub(1))
    }

    fn tune(&mut self) {
        let lo = self.p.v(P_LO);
        let hi = self.p.v(P_HI).max(lo * 1.1);
        if (lo, hi) == self.tuned {
            return;
        }
        self.tuned = (lo, hi);
        let sr = self.sr;
        for c in self.chans.iter_mut() {
            c.det_hp.iter_mut().for_each(|b| b.set(true, lo, sr));
            c.det_lp.set(false, hi, sr);
            c.rest_lp.set(false, lo * 0.5, sr);
            c.split.iter_mut().for_each(|b| b.set(false, lo, sr));
        }
    }

    fn run(&mut self, io: &mut [Vec<f32>], frames: usize) {
        let nch = io.len().min(self.ch).min(2);
        if nch == 0 {
            return;
        }
        let n = frames.min(io[..nch].iter().map(Vec::len).min().unwrap_or(0));
        self.tune();
        let vocal = self.p.choice(P_MODE) == VOCAL;
        let split = self.p.choice(P_BAND) == SPLIT;
        let thr = self.p.v(P_THR);
        let range = self.p.v(P_RANGE);
        let link = self.p.v(P_LINK) * 0.01;
        let listen = self.p.on(P_LISTEN);
        let la = self.lookahead_samples();
        let sr = self.sr;
        let env_fall = (-1000.0 / (8.0 * sr)).exp();
        let att = ms_coef(if la > 0 { (la as f32 / sr * 1000.0 * 0.5).max(0.2) } else { 0.2 }, sr);
        let rel = ms_coef(RELEASE_MS, sr);
        let dlen = self.delay[0].len();
        let mut col_in = 0.0f32;
        let mut col_out = 0.0f32;
        let mut col_gr = 0.0f32;
        let mut col_det = 0.0f32;
        for i in 0..n {
            let mut det = [0.0f32; 2];
            let mut heard = [0.0f32; 2];
            for c in 0..nch {
                let x = io[c][i];
                let ch = &mut self.chans[c];
                let mut b = x;
                for f in ch.det_hp.iter_mut() {
                    b = f.run(b);
                }
                b = ch.det_lp.run(b);
                heard[c] = b;
                ch.band_env = b.abs().max(ch.band_env * env_fall);
                let band_db = crate::gain_to_db(ch.band_env.max(1.0e-9));
                det[c] = if vocal {
                    let r = ch.rest_lp.run(x);
                    ch.rest_env = r.abs().max(ch.rest_env * env_fall);
                    let rest_db = crate::gain_to_db(ch.rest_env.max(1.0e-9));
                    band_db + (band_db - rest_db).min(0.0)
                } else {
                    band_db
                };
            }
            if nch == 1 {
                det[1] = det[0];
            }
            let linked = det[0].max(det[1]);
            col_det = col_det.max(linked);
            let mut gr_max = 0.0f32;
            for c in 0..nch {
                let d = link * linked + (1.0 - link) * det[c];
                let target = reduction(d, thr, range);
                let ch = &mut self.chans[c];
                let k = if target > ch.gr { att } else { rel };
                ch.gr += k * (target - ch.gr);
                gr_max = gr_max.max(ch.gr);
            }
            for c in 0..nch {
                let x = io[c][i];
                col_in = col_in.max(x.abs());
                let line = &mut self.delay[c];
                line[self.dpos] = x;
                let dry = line[(self.dpos + dlen - la) % dlen];
                let ch = &mut self.chans[c];
                let g = crate::db_to_gain(-ch.gr);
                let y = if listen {
                    heard[c]
                } else if split {
                    let mut lo = dry;
                    for f in ch.split.iter_mut() {
                        lo = f.run(lo);
                    }
                    lo + (dry - lo) * g
                } else {
                    dry * g
                };
                col_out = col_out.max(y.abs());
                io[c][i] = y;
            }
            self.dpos = (self.dpos + 1) % dlen;
            self.gr_now = gr_max;
            col_gr = col_gr.max(gr_max);
            self.col_n += 1;
            if self.col_n >= self.col_len {
                self.col_n = 0;
                let col = Column {
                    input: crate::gain_to_db(col_in.max(1.0e-7)),
                    output: crate::gain_to_db(col_out.max(1.0e-7)),
                    gr: col_gr,
                    aux: col_det.max(-140.0),
                    hit: f32::NEG_INFINITY,
                };
                if let Some(mut d) = self.scope.try_write() {
                    d.rate = self.sr;
                    d.column_secs = self.col_len as f32 / self.sr;
                    d.push_column(col);
                    d.readout = col_gr;
                }
                col_in = 0.0;
                col_out = 0.0;
                col_gr = 0.0;
                col_det = -140.0;
            }
        }
    }
}

impl Plugin for OpenDs {
    param_plumbing!(OPEN_DS_INFO);

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = if sample_rate.is_finite() && sample_rate > 0.0 { sample_rate.clamp(8_000.0, 384_000.0) } else { 48_000.0 };
        self.ch = channels.clamp(1, 2);
        let len = (MAX_LOOKAHEAD_MS * 0.001 * self.sr) as usize + 2;
        self.delay = [vec![0.0; len], vec![0.0; len]];
        self.dpos = 0;
        self.chans = [Chan::default(); 2];
        self.tuned = (0.0, 0.0);
        self.tune();
        self.col_len = ((0.01 * self.sr) as usize).max(1);
        self.col_n = 0;
    }

    fn reset(&mut self) {
        self.chans.iter_mut().for_each(Chan::clear);
        for d in self.delay.iter_mut() {
            d.iter_mut().for_each(|v| *v = 0.0);
        }
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        self.run(io, frames);
    }

    fn latency(&self) -> usize {
        self.lookahead_samples()
    }

    fn gain_reduction_db(&self) -> f32 {
        self.gr_now
    }

    fn scope(&self) -> Option<Arc<Scope>> {
        Some(Arc::clone(&self.scope))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_tone(ds: &mut OpenDs, f: f32, amp: f32) -> (f32, f32) {
        let mut out = 0.0f32;
        for blk in 0..60 {
            let x: Vec<f32> = (0..512).map(|i| amp * (std::f32::consts::TAU * f * (blk * 512 + i) as f32 / 48_000.0).sin()).collect();
            let mut io = vec![x.clone(), x];
            ds.process(&mut io, 512);
            assert!(io[0].iter().all(|v| v.is_finite()));
            out = io[0].iter().fold(0.0, |m, v| m.max(v.abs()));
        }
        (out, ds.gain_reduction_db())
    }

    fn fresh(mode: usize, band: usize) -> OpenDs {
        let mut ds = OpenDs::new();
        ds.set_param("mode", mode as f32);
        ds.set_param("band", band as f32);
        ds.set_param("threshold", -30.0);
        ds.set_param("range", 12.0);
        ds.prepare(48_000.0, 512, 2);
        ds
    }

    #[test]
    fn sibilance_is_reduced_and_low_end_left_alone() {
        for mode in 0..MODES.len() {
            let mut ds = fresh(mode, SPLIT);
            let (hi, gr) = run_tone(&mut ds, 10_000.0, 0.5);
            assert!(gr > 10.0 && hi < 0.5 * 0.4, "mode {mode}: 10 kHz out {hi}, gr {gr}");
            let mut ds = fresh(mode, SPLIT);
            let (lo, gr) = run_tone(&mut ds, 300.0, 0.5);
            assert!(gr < 0.5 && (lo - 0.5).abs() < 0.02, "mode {mode}: 300 Hz out {lo}, gr {gr}");
        }
    }

    #[test]
    fn single_vocal_ignores_bright_vowels() {
        let mut ds = fresh(VOCAL, SPLIT);
        let mut gr = 0.0f32;
        for blk in 0..60 {
            let x: Vec<f32> = (0..512)
                .map(|i| {
                    let t = (blk * 512 + i) as f32 / 48_000.0;
                    0.8 * (std::f32::consts::TAU * 250.0 * t).sin() + 0.15 * (std::f32::consts::TAU * 6_000.0 * t).sin()
                })
                .collect();
            let mut io = vec![x.clone(), x];
            ds.process(&mut io, 512);
            gr = ds.gain_reduction_db();
        }
        let mut all = fresh(1, SPLIT);
        let mut gr_all = 0.0f32;
        for blk in 0..60 {
            let x: Vec<f32> = (0..512)
                .map(|i| {
                    let t = (blk * 512 + i) as f32 / 48_000.0;
                    0.8 * (std::f32::consts::TAU * 250.0 * t).sin() + 0.15 * (std::f32::consts::TAU * 6_000.0 * t).sin()
                })
                .collect();
            let mut io = vec![x.clone(), x];
            all.process(&mut io, 512);
            gr_all = all.gain_reduction_db();
        }
        assert!(gr + 3.0 < gr_all, "vocal {gr} allround {gr_all}");
    }

    #[test]
    fn split_band_sums_back_to_the_input_when_idle() {
        let mut ds = fresh(VOCAL, SPLIT);
        ds.set_param("threshold", 0.0);
        ds.set_param("lookahead", 0.0);
        let x: Vec<f32> = (0..4096).map(|i| ((i * 7919) % 1000) as f32 / 1000.0 - 0.5).collect();
        let mut io = vec![x.iter().map(|v| v * 0.01).collect::<Vec<_>>(); 2];
        ds.process(&mut io, 4096);
        let err = io[0].iter().zip(&x).map(|(y, x)| (y - x * 0.01).abs()).fold(0.0, f32::max);
        assert!(err < 1.0e-5, "{err}");
    }
}
