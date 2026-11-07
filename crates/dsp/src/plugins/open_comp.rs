use crate::params::{Params, choice, lin, log, param_plumbing, toggle};
use crate::scope::{Column, Scope};
use crate::{Category, ParamInfo, Plugin, PluginInfo, Unit};
use std::sync::Arc;

pub const STYLES: &[&str] = &["Clean", "Classic", "Opto", "Vocal", "Mastering", "Bus", "Punch", "Pumping"];
pub const CLEAN: usize = 0;
pub const CLASSIC: usize = 1;
pub const OPTO: usize = 2;
pub const VOCAL: usize = 3;
pub const MASTERING: usize = 4;
pub const BUS: usize = 5;
pub const PUNCH: usize = 6;
pub const PUMPING: usize = 7;

const P_STYLE: usize = 0;
const P_THR: usize = 1;
const P_RATIO: usize = 2;
const P_KNEE: usize = 3;
const P_RANGE: usize = 4;
const P_ATTACK: usize = 5;
const P_RELEASE: usize = 6;
const P_AUTO_REL: usize = 7;
const P_LOOKAHEAD: usize = 8;
const P_HOLD: usize = 9;
const P_GAIN: usize = 10;
const P_AUTO_GAIN: usize = 11;
const P_MIX: usize = 12;
const P_LINK: usize = 13;
const P_SC_HP: usize = 14;
const P_SC_LP: usize = 15;
const P_AUDITION: usize = 16;

static PARAMS: [ParamInfo; 17] = [
    choice("style", "Style", STYLES, 0),
    lin("threshold", "Threshold", -60.0, 0.0, -18.0, Unit::Db),
    log("ratio", "Ratio", 1.0, 100.0, 4.0, Unit::Ratio),
    lin("knee", "Knee", 0.0, 72.0, 12.0, Unit::Db),
    lin("range", "Range", 0.0, 60.0, 60.0, Unit::Db),
    log("attack", "Attack", 0.005, 250.0, 10.0, Unit::Ms),
    log("release", "Release", 10.0, 2500.0, 100.0, Unit::Ms),
    toggle("auto_release", "Auto Release", false),
    lin("lookahead", "Lookahead", 0.0, 20.0, 0.0, Unit::Ms),
    lin("hold", "Hold", 0.0, 500.0, 0.0, Unit::Ms),
    lin("gain", "Gain", -36.0, 36.0, 0.0, Unit::Db),
    toggle("auto_gain", "Auto Gain", false),
    lin("mix", "Dry/Wet", 0.0, 100.0, 100.0, Unit::Percent),
    lin("link", "Stereo Link", 0.0, 100.0, 100.0, Unit::Percent),
    log("sc_hp", "Side-Chain High Pass", 10.0, 20000.0, 10.0, Unit::Hz),
    log("sc_lp", "Side-Chain Low Pass", 10.0, 20000.0, 20000.0, Unit::Hz),
    toggle("audition", "Audition Side-Chain", false),
];

pub static OPEN_COMP_INFO: PluginInfo = PluginInfo {
    id: "open_comp",
    name: "OpenComp",
    short_name: "OpenComp",
    category: Category::Dynamics,
    params: &PARAMS,
    is_instrument: false,
    audiosuite: true,
};

pub fn static_gr(lvl: f32, thr: f32, ratio: f32, knee: f32, range: f32) -> f32 {
    let slope = if ratio >= 99.9 { 1.0 } else { 1.0 - 1.0 / ratio.max(1.0) };
    let over = lvl - thr;
    let w = knee.max(0.0);
    let gr = if 2.0 * over < -w {
        0.0
    } else if w > 0.0 && 2.0 * over <= w {
        let t = over + 0.5 * w;
        slope * t * t / (2.0 * w)
    } else {
        slope * over
    };
    soft_cap(gr.max(0.0), range)
}

fn soft_cap(gr: f32, range: f32) -> f32 {
    let r = range.max(0.0);
    let w = (r * 0.25).min(3.0);
    if w <= 1.0e-3 || gr <= r - w {
        return gr.min(r);
    }
    if gr >= r + w {
        return r;
    }
    let t = gr - (r - w);
    gr - t * t / (4.0 * w)
}

#[derive(Clone, Copy)]
struct StyleTraits {
    rms_ms: f32,
    feedback: bool,
    min_knee: f32,
    program_release: bool,
    linear_release: bool,
    soft_attack: bool,
    ratio_boost: f32,
}

fn traits(style: usize) -> StyleTraits {
    let base = StyleTraits {
        rms_ms: 0.0,
        feedback: false,
        min_knee: 0.0,
        program_release: false,
        linear_release: false,
        soft_attack: false,
        ratio_boost: 1.0,
    };
    match style {
        CLASSIC => StyleTraits { feedback: true, rms_ms: 1.0, ..base },
        OPTO => StyleTraits { feedback: true, rms_ms: 5.0, min_knee: 12.0, program_release: true, ..base },
        VOCAL => StyleTraits { rms_ms: 10.0, min_knee: 12.0, ratio_boost: 1.6, ..base },
        MASTERING => StyleTraits { rms_ms: 30.0, min_knee: 6.0, ..base },
        BUS => StyleTraits { rms_ms: 20.0, program_release: true, ..base },
        PUNCH => StyleTraits { soft_attack: true, ..base },
        PUMPING => StyleTraits { linear_release: true, ..base },
        _ => base,
    }
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
    gr: f32,
    gr2: f32,
    slow: f32,
    hold: u32,
    fb: f32,
    att_state: f32,
}

const MAX_LOOKAHEAD_MS: f32 = 20.0;

pub struct OpenComp {
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
    col_len: usize,
    col_n: usize,
    col: Column,
    scope: Arc<Scope>,
}

impl Default for OpenComp {
    fn default() -> Self {
        Self::new()
    }
}

fn ms_coef(ms: f32, sr: f32) -> f32 {
    if ms <= 0.0 { 1.0 } else { 1.0 - (-1000.0 / (ms * sr)).exp() }
}

impl OpenComp {
    pub fn new() -> OpenComp {
        OpenComp {
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
            col_len: 480,
            col_n: 0,
            col: Column { input: -120.0, output: -120.0, gr: 0.0, aux: -120.0, hit: f32::NEG_INFINITY },
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
        let thr = self.p.v(P_THR);
        let ratio = (self.p.v(P_RATIO) * tr.ratio_boost).min(100.0);
        let knee = self.p.v(P_KNEE).max(tr.min_knee);
        let range = self.p.v(P_RANGE);
        let attack_ms = self.p.v(P_ATTACK);
        let release_ms = self.p.v(P_RELEASE);
        let auto_rel = self.p.on(P_AUTO_REL);
        let hold_n = (self.p.v(P_HOLD) * 0.001 * self.sr) as u32;
        let la = self.lookahead_samples();
        let link = self.p.v(P_LINK) * 0.01;
        let use_hp = self.sc_hp > 10.5;
        let use_lp = self.sc_lp < 19_500.0;
        let audition = self.p.on(P_AUDITION);
        let sr = self.sr;
        let auto_db = if self.p.on(P_AUTO_GAIN) { (0.5 * (-thr) * if ratio >= 99.9 { 1.0 } else { 1.0 - 1.0 / ratio }).min(range) } else { 0.0 };
        let makeup_t = crate::db_to_gain(self.p.v(P_GAIN) + auto_db);
        let mix_t = self.p.v(P_MIX) * 0.01;
        let smooth = 1.0 - (-1.0 / (0.03 * sr)).exp();
        let att = ms_coef(attack_ms, sr);
        let rel = ms_coef(release_ms, sr);
        let rel_fast = ms_coef(release_ms * 0.3, sr);
        let rel_slow = ms_coef(release_ms * 2.5, sr);
        let slow_att = ms_coef(release_ms, sr);
        let rms_c = if tr.rms_ms > 0.0 { ms_coef(tr.rms_ms, sr) } else { 1.0 };
        #[allow(clippy::manual_clamp)]
        let lin_rel_db = range.max(6.0).min(24.0) * 1000.0 / (release_ms.max(10.0) * sr);
        let dlen = self.delay[0].len();
        let mut col_in = f32::NEG_INFINITY;
        let mut col_out = f32::NEG_INFINITY;
        let mut col_gr = 0.0f32;
        for i in 0..n {
            let mut det = [0.0f32; 2];
            for c in 0..nch.max(1) {
                let x = match key.filter(|k| !k.is_empty()) {
                    Some(k) => k[c.min(k.len() - 1)].get(i).copied().unwrap_or(0.0),
                    None => io[c][i],
                };
                let ch = &mut self.chans[c];
                let mut s = if tr.feedback { ch.fb } else { x };
                if use_hp {
                    s = ch.hp.run(s);
                }
                if use_lp {
                    s = ch.lp.run(s);
                }
                if audition {
                    det[c] = s;
                }
                ch.rms += rms_c * (s * s - ch.rms);
                let lvl = if tr.rms_ms > 0.0 { (2.0 * ch.rms).sqrt() } else { s.abs() };
                if !audition {
                    det[c] = lvl;
                }
            }
            if nch == 1 {
                det[1] = det[0];
            }
            let peak_in = io[..nch].iter().map(|c| c[i].abs()).fold(0.0, f32::max);
            col_in = col_in.max(peak_in);
            let linked = det[0].abs().max(det[1].abs());
            for c in 0..nch {
                let own = if audition { 0.0 } else { det[c] };
                let lvl_lin = if audition { 0.0 } else { link * linked + (1.0 - link) * own };
                let lvl = crate::gain_to_db(lvl_lin.max(1.0e-9));
                let target = static_gr(lvl, thr, ratio, knee, range);
                let ch = &mut self.chans[c];
                if target > ch.gr {
                    let a = if tr.soft_attack {
                        ch.att_state += att * (target - ch.att_state);
                        att * (0.25 + 0.75 * (ch.att_state / target.max(1.0e-3)).clamp(0.0, 1.0))
                    } else {
                        att
                    };
                    ch.gr += a * (target - ch.gr);
                    ch.hold = hold_n;
                } else if ch.hold > 0 {
                    ch.hold -= 1;
                } else if tr.linear_release {
                    ch.gr = (ch.gr - lin_rel_db).max(target);
                } else if auto_rel || tr.program_release {
                    let r = rel_fast * (1.0 - 0.7 * (ch.slow / range.max(1.0)).clamp(0.0, 1.0));
                    ch.gr += r * (target - ch.gr);
                } else {
                    ch.gr += rel * (target - ch.gr);
                }
                if target < ch.att_state {
                    ch.att_state += rel * (target - ch.att_state);
                }
                let c_slow = if ch.gr > ch.slow { slow_att } else { rel_slow };
                ch.slow += c_slow * (ch.gr - ch.slow);
                let gr = if auto_rel || tr.program_release { ch.gr.max(ch.slow * 0.9) } else { ch.gr };
                ch.gr2 = gr;
            }
            if nch == 1 {
                self.chans[1].gr2 = self.chans[0].gr2;
            }
            self.makeup += smooth * (makeup_t - self.makeup);
            self.mix += smooth * (mix_t - self.mix);
            let mut gr_max = 0.0f32;
            let mut peak_out = 0.0f32;
            for c in 0..nch {
                let x = io[c][i];
                let d = &mut self.delay[c];
                d[self.dpos] = x;
                let dry = d[(self.dpos + dlen - la) % dlen];
                let gr = self.chans[c].gr2;
                gr_max = gr_max.max(gr);
                let wet = dry * crate::db_to_gain(-gr);
                self.chans[c].fb = wet;
                let y = if audition { det[c] } else { (dry + self.mix * (wet - dry)) * self.makeup };
                peak_out = peak_out.max(y.abs());
                io[c][i] = y;
            }
            self.dpos = (self.dpos + 1) % dlen;
            col_out = col_out.max(peak_out);
            col_gr = col_gr.max(gr_max);
            self.gr_now = gr_max;
            self.col_n += 1;
            if self.col_n >= self.col_len {
                self.col_n = 0;
                self.col = Column {
                    input: crate::gain_to_db(col_in.max(1.0e-7)),
                    output: crate::gain_to_db(col_out.max(1.0e-7)),
                    gr: col_gr,
                    aux: crate::gain_to_db(linked.max(1.0e-7)),
                    hit: f32::NEG_INFINITY,
                };
                if let Some(mut d) = self.scope.try_write() {
                    d.rate = self.sr;
                    d.column_secs = self.col_len as f32 / self.sr;
                    d.key_live = key.is_some_and(|k| !k.is_empty());
                    d.push_column(self.col);
                    d.readout = self.gr_now;
                }
                col_in = f32::NEG_INFINITY;
                col_out = f32::NEG_INFINITY;
                col_gr = 0.0;
            }
        }
    }
}

impl Plugin for OpenComp {
    param_plumbing!(OPEN_COMP_INFO);

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

    #[test]
    fn static_curve_and_steady_state() {
        assert!((static_gr(-6.0, -18.0, 4.0, 0.0, 60.0) - 9.0).abs() < 1.0e-4);
        assert_eq!(static_gr(-30.0, -18.0, 4.0, 6.0, 60.0), 0.0);
        assert!(static_gr(0.0, -40.0, 100.0, 0.0, 12.0) <= 12.0);
        for style in 0..STYLES.len() {
            let mut c = OpenComp::new();
            c.set_param("style", style as f32);
            c.set_param("threshold", -20.0);
            c.prepare(48_000.0, 512, 2);
            let mut gr = 0.0;
            for blk in 0..200 {
                let x: Vec<f32> = (0..512).map(|i| 0.5 * (std::f32::consts::TAU * 200.0 * (blk * 512 + i) as f32 / 48_000.0).sin()).collect();
                let mut io = vec![x.clone(), x];
                c.process(&mut io, 512);
                assert!(io[0].iter().all(|v| v.is_finite()));
                gr = c.gain_reduction_db();
            }
            assert!(gr > 3.0, "style {style}: {gr}");
        }
    }
}
