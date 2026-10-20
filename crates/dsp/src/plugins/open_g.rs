use crate::params::{Params, choice, lin, log, param_plumbing, toggle};
use crate::scope::{Column, Scope};
use crate::{Category, ParamInfo, Plugin, PluginInfo, Unit};
use std::sync::Arc;

pub const STYLES: &[&str] = &["Classic", "Clipper", "Vocal", "Guitar", "Upward", "Ducking"];
pub const CLASSIC: usize = 0;
pub const CLIPPER: usize = 1;
pub const VOCAL: usize = 2;
pub const GUITAR: usize = 3;
pub const UPWARD: usize = 4;
pub const DUCKING: usize = 5;

const P_STYLE: usize = 0;
const P_THR: usize = 1;
const P_RATIO: usize = 2;
const P_RANGE: usize = 3;
const P_KNEE: usize = 4;
const P_ATTACK: usize = 5;
const P_HOLD: usize = 6;
const P_RELEASE: usize = 7;
const P_LOOKAHEAD: usize = 8;
const P_GAIN: usize = 9;
const P_MIX: usize = 10;
const P_LINK: usize = 11;
const P_SC_HP: usize = 12;
const P_SC_LP: usize = 13;
const P_AUDITION: usize = 14;

static PARAMS: [ParamInfo; 15] = [
    choice("style", "Style", STYLES, 0),
    lin("threshold", "Threshold", -80.0, 0.0, -40.0, Unit::Db),
    log("ratio", "Ratio", 1.0, 100.0, 100.0, Unit::Ratio),
    lin("range", "Range", 0.0, 100.0, 40.0, Unit::Db),
    lin("knee", "Knee", 0.0, 30.0, 0.0, Unit::Db),
    log("attack", "Attack", 0.005, 250.0, 0.5, Unit::Ms),
    lin("hold", "Hold", 0.0, 500.0, 50.0, Unit::Ms),
    log("release", "Release", 1.0, 5000.0, 100.0, Unit::Ms),
    lin("lookahead", "Lookahead", 0.0, 10.0, 0.0, Unit::Ms),
    lin("gain", "Gain", -36.0, 36.0, 0.0, Unit::Db),
    lin("mix", "Dry/Wet", 0.0, 100.0, 100.0, Unit::Percent),
    lin("link", "Stereo Link", 0.0, 100.0, 100.0, Unit::Percent),
    log("sc_hp", "Side-Chain High Pass", 10.0, 20000.0, 10.0, Unit::Hz),
    log("sc_lp", "Side-Chain Low Pass", 10.0, 20000.0, 20000.0, Unit::Hz),
    toggle("audition", "Audition Side-Chain", false),
];

pub static OPEN_G_INFO: PluginInfo = PluginInfo {
    id: "open_g",
    name: "OpenG",
    short_name: "OpenG",
    category: Category::Dynamics,
    params: &PARAMS,
    is_instrument: false,
    audiosuite: true,
};

#[derive(Clone, Copy)]
pub struct StyleTraits {
    rms_ms: f32,
    peak_ms: f32,
    pub hysteresis: f32,
    pub min_knee: f32,
    instant: bool,
}

pub fn traits(style: usize) -> StyleTraits {
    let base = StyleTraits { rms_ms: 0.0, peak_ms: 10.0, hysteresis: 0.0, min_knee: 0.0, instant: false };
    match style {
        CLIPPER => StyleTraits { peak_ms: 0.0, instant: true, ..base },
        VOCAL => StyleTraits { rms_ms: 10.0, peak_ms: 0.0, hysteresis: 2.0, min_knee: 6.0, ..base },
        GUITAR => StyleTraits { peak_ms: 4.0, hysteresis: 6.0, ..base },
        _ => base,
    }
}

pub fn boosts(style: usize) -> bool {
    style == UPWARD
}

pub fn static_gain(style: usize, lvl: f32, thr: f32, ratio: f32, knee: f32, range: f32) -> f32 {
    let slope = if ratio >= 99.9 { 1000.0 } else { ratio.max(1.0) - 1.0 };
    let x = if matches!(style, UPWARD | DUCKING) { lvl - thr } else { thr - lvl };
    let w = knee.max(traits(style).min_knee);
    let act = if 2.0 * x < -w {
        0.0
    } else if w > 0.0 && 2.0 * x <= w {
        let t = x + 0.5 * w;
        slope * t * t / (2.0 * w)
    } else {
        slope * x
    };
    let a = act.max(0.0).min(range.max(0.0));
    if boosts(style) { a } else { -a }
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
        let w0 = std::f32::consts::TAU * f.clamp(5.0, sr * 0.45) / sr;
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
}

#[derive(Debug, Clone, Copy, Default)]
struct Chan {
    hp: Biquad,
    lp: Biquad,
    rms: f32,
    peak: f32,
    act: f32,
    hold: u32,
    open: bool,
}

const MAX_LOOKAHEAD_MS: f32 = 10.0;

pub struct OpenG {
    p: Params,
    sr: f32,
    ch: usize,
    chans: [Chan; 2],
    delay: [Vec<f32>; 2],
    dpos: usize,
    makeup: f32,
    mix: f32,
    sc_hp: f32,
    sc_lp: f32,
    gr_now: f32,
    gain_now: f32,
    col_len: usize,
    col_n: usize,
    scope: Arc<Scope>,
}

impl Default for OpenG {
    fn default() -> Self {
        Self::new()
    }
}

fn ms_coef(ms: f32, sr: f32) -> f32 {
    if ms <= 0.0 { 1.0 } else { 1.0 - (-1000.0 / (ms * sr)).exp() }
}

impl OpenG {
    pub fn new() -> OpenG {
        OpenG {
            p: Params::new(&PARAMS),
            sr: 48_000.0,
            ch: 2,
            chans: [Chan::default(); 2],
            delay: [Vec::new(), Vec::new()],
            dpos: 0,
            makeup: 1.0,
            mix: 1.0,
            sc_hp: 0.0,
            sc_lp: 0.0,
            gr_now: 0.0,
            gain_now: 0.0,
            col_len: 480,
            col_n: 0,
            scope: Scope::new(),
        }
    }

    fn update(&mut self) {}

    fn lookahead_samples(&self) -> usize {
        ((self.p.v(P_LOOKAHEAD).clamp(0.0, MAX_LOOKAHEAD_MS) * 0.001 * self.sr).round() as usize).min(self.delay[0].len().saturating_sub(1))
    }

    fn filters(&mut self) {
        let (hp, lp) = (self.p.v(P_SC_HP), self.p.v(P_SC_LP));
        if hp != self.sc_hp || lp != self.sc_lp {
            self.sc_hp = hp;
            self.sc_lp = lp;
            for c in self.chans.iter_mut() {
                c.hp.set(true, hp, self.sr);
                c.lp.set(false, lp, self.sr);
            }
        }
    }

    fn run(&mut self, io: &mut [Vec<f32>], key: Option<&[Vec<f32>]>, frames: usize) {
        let nch = io.len().min(self.ch).min(2);
        if nch == 0 {
            return;
        }
        let n = frames.min(io[..nch].iter().map(Vec::len).min().unwrap_or(0));
        self.filters();
        let style = self.p.choice(P_STYLE);
        let tr = traits(style);
        let up = boosts(style);
        let engage_up = matches!(style, UPWARD | DUCKING);
        let thr = self.p.v(P_THR);
        let ratio = self.p.v(P_RATIO);
        let knee = self.p.v(P_KNEE);
        let range = self.p.v(P_RANGE);
        let hold_n = (self.p.v(P_HOLD) * 0.001 * self.sr) as u32;
        let la = self.lookahead_samples();
        let link = self.p.v(P_LINK) * 0.01;
        let use_hp = self.sc_hp > 10.5;
        let use_lp = self.sc_lp < 19_500.0;
        let audition = self.p.on(P_AUDITION);
        let sr = self.sr;
        let makeup_t = crate::db_to_gain(self.p.v(P_GAIN));
        let mix_t = self.p.v(P_MIX) * 0.01;
        let smooth = 1.0 - (-1.0 / (0.03 * sr)).exp();
        let att = if tr.instant { 1.0 } else { ms_coef(self.p.v(P_ATTACK), sr) };
        let rel = ms_coef(self.p.v(P_RELEASE), sr);
        let rms_c = if tr.rms_ms > 0.0 { ms_coef(tr.rms_ms, sr) } else { 1.0 };
        let peak_fall = if tr.peak_ms > 0.0 { (-1000.0 / (tr.peak_ms * sr)).exp() } else { 0.0 };
        let dlen = self.delay[0].len();
        let mut col_in = f32::NEG_INFINITY;
        let mut col_out = f32::NEG_INFINITY;
        let mut col_gr = 0.0f32;
        let mut col_det = 0.0f32;
        for i in 0..n {
            let mut det = [0.0f32; 2];
            let mut heard = [0.0f32; 2];
            for c in 0..nch {
                let x = match key.filter(|k| !k.is_empty()) {
                    Some(k) => k[c.min(k.len() - 1)].get(i).copied().unwrap_or(0.0),
                    None => io[c][i],
                };
                let ch = &mut self.chans[c];
                let mut s = x;
                if use_hp {
                    s = ch.hp.run(s);
                }
                if use_lp {
                    s = ch.lp.run(s);
                }
                heard[c] = s;
                let lvl = if tr.rms_ms > 0.0 {
                    ch.rms += rms_c * (s * s - ch.rms);
                    (2.0 * ch.rms).sqrt()
                } else {
                    ch.peak = s.abs().max(ch.peak * peak_fall);
                    ch.peak
                };
                det[c] = lvl;
            }
            if nch == 1 {
                det[1] = det[0];
                heard[1] = heard[0];
            }
            let peak_in = io[..nch].iter().map(|c| c[i].abs()).fold(0.0, f32::max);
            col_in = col_in.max(peak_in);
            let linked = det[0].max(det[1]);
            col_det = col_det.max(linked);
            for c in 0..nch {
                let lvl = crate::gain_to_db((link * linked + (1.0 - link) * det[c]).max(1.0e-9));
                let ch = &mut self.chans[c];
                let above = lvl > thr;
                if above {
                    ch.open = true;
                } else if lvl < thr - tr.hysteresis {
                    ch.open = false;
                }
                let t_eff = if ch.open && !engage_up { thr - tr.hysteresis } else { thr };
                let target = static_gain(style, lvl, t_eff, ratio, knee, range).abs();
                let engaging = if engage_up { target >= ch.act } else { target <= ch.act };
                if engaging {
                    ch.act += att * (target - ch.act);
                    ch.hold = hold_n;
                } else if ch.hold > 0 {
                    ch.hold -= 1;
                } else {
                    ch.act += rel * (target - ch.act);
                }
            }
            if nch == 1 {
                self.chans[1].act = self.chans[0].act;
            }
            self.makeup += smooth * (makeup_t - self.makeup);
            self.mix += smooth * (mix_t - self.mix);
            let mut act_max = 0.0f32;
            let mut peak_out = 0.0f32;
            for c in 0..nch {
                let x = io[c][i];
                let d = &mut self.delay[c];
                d[self.dpos] = x;
                let dry = d[(self.dpos + dlen - la) % dlen];
                let act = self.chans[c].act;
                act_max = act_max.max(act);
                let wet = dry * crate::db_to_gain(if up { act } else { -act });
                let y = if audition { heard[c] } else { (dry + self.mix * (wet - dry)) * self.makeup };
                peak_out = peak_out.max(y.abs());
                io[c][i] = y;
            }
            self.dpos = (self.dpos + 1) % dlen;
            col_out = col_out.max(peak_out);
            col_gr = col_gr.max(act_max);
            self.gr_now = if up { 0.0 } else { act_max };
            self.gain_now = if up { act_max } else { -act_max };
            self.col_n += 1;
            if self.col_n >= self.col_len {
                self.col_n = 0;
                let col = Column {
                    input: crate::gain_to_db(col_in.max(1.0e-7)),
                    output: crate::gain_to_db(col_out.max(1.0e-7)),
                    gr: col_gr,
                    aux: crate::gain_to_db(col_det.max(1.0e-7)),
                    hit: f32::NEG_INFINITY,
                };
                if let Some(mut d) = self.scope.try_write() {
                    d.rate = self.sr;
                    d.column_secs = self.col_len as f32 / self.sr;
                    d.key_live = key.is_some_and(|k| !k.is_empty());
                    d.push_column(col);
                    d.readout = self.gain_now;
                }
                col_in = f32::NEG_INFINITY;
                col_out = f32::NEG_INFINITY;
                col_gr = 0.0;
                col_det = 0.0;
            }
        }
    }
}

impl Plugin for OpenG {
    param_plumbing!(OPEN_G_INFO);

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = if sample_rate.is_finite() && sample_rate > 0.0 { sample_rate.clamp(8_000.0, 384_000.0) } else { 48_000.0 };
        self.ch = channels.clamp(1, 2);
        let len = (MAX_LOOKAHEAD_MS * 0.001 * self.sr) as usize + 2;
        self.delay = [vec![0.0; len], vec![0.0; len]];
        self.dpos = 0;
        self.chans = [Chan::default(); 2];
        self.sc_hp = 0.0;
        self.sc_lp = 0.0;
        self.filters();
        self.makeup = crate::db_to_gain(self.p.v(P_GAIN));
        self.mix = self.p.v(P_MIX) * 0.01;
        self.col_len = ((0.01 * self.sr) as usize).max(1);
        self.col_n = 0;
    }

    fn reset(&mut self) {
        for c in self.chans.iter_mut() {
            *c = Chan { hp: c.hp, lp: c.lp, ..Chan::default() };
        }
        for d in self.delay.iter_mut() {
            d.iter_mut().for_each(|v| *v = 0.0);
        }
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

    fn tone(amp: f32, blk: usize) -> Vec<f32> {
        (0..512).map(|i| amp * (std::f32::consts::TAU * 200.0 * (blk * 512 + i) as f32 / 48_000.0).sin()).collect()
    }

    fn settle(g: &mut OpenG, amp: f32) -> f32 {
        let mut out = 0.0f32;
        for blk in 0..100 {
            let x = tone(amp, blk);
            let mut io = vec![x.clone(), x];
            g.process(&mut io, 512);
            assert!(io[0].iter().all(|v| v.is_finite()));
            out = io[0].iter().fold(0.0, |m, v| m.max(v.abs()));
        }
        out
    }

    #[test]
    fn static_curves() {
        assert_eq!(static_gain(CLASSIC, -20.0, -40.0, 100.0, 0.0, 40.0), 0.0);
        assert_eq!(static_gain(CLASSIC, -60.0, -40.0, 100.0, 0.0, 40.0), -40.0);
        assert!((static_gain(CLASSIC, -45.0, -40.0, 2.0, 0.0, 40.0) + 5.0).abs() < 1.0e-4);
        assert_eq!(static_gain(DUCKING, -20.0, -40.0, 100.0, 0.0, 12.0), -12.0);
        assert_eq!(static_gain(DUCKING, -60.0, -40.0, 100.0, 0.0, 12.0), 0.0);
        assert_eq!(static_gain(UPWARD, -20.0, -40.0, 100.0, 0.0, 6.0), 6.0);
    }

    #[test]
    fn every_style_opens_above_and_acts_below() {
        for style in 0..STYLES.len() {
            let mut g = OpenG::new();
            g.set_param("style", style as f32);
            g.set_param("threshold", -30.0);
            g.set_param("range", 40.0);
            g.prepare(48_000.0, 512, 2);
            let loud = settle(&mut g, 0.5);
            let quiet = settle(&mut g, 0.005);
            match style {
                UPWARD => assert!(loud > 0.9, "upward boosts: {loud}"),
                DUCKING => assert!(loud < 0.05 && quiet > 0.004, "ducking: {loud} {quiet}"),
                _ => assert!(loud > 0.45 && quiet < 0.0002, "style {style}: {loud} {quiet}"),
            }
        }
    }
}
