use crate::biquad::{Biquad, FilterType};
use crate::params::{Params, choice, lin, log, param_plumbing, toggle};
use crate::scope::Scope;
use crate::util::{DelayLine, Lfo, Rng, Smoother, flush, frames_in, ms_to_samples, sane_channels, sane_sr};
use crate::{Category, ParamInfo, Plugin, PluginInfo, Transport, Unit};
use std::sync::Arc;

pub const STYLES: &[&str] = &["Clean", "Studio Tape", "Tape Echo", "Analog", "Lo-Fi", "Ambient"];
pub const MODES: &[&str] = &["Single", "Dual", "Ping-Pong"];
pub const SINGLE: usize = 0;
pub const DUAL: usize = 1;
pub const PING_PONG: usize = 2;
pub const NOTES: &[&str] =
    &["1/64", "1/32T", "1/32", "1/16T", "1/32D", "1/16", "1/8T", "1/16D", "1/8", "1/4T", "1/8D", "1/4", "1/2T", "1/4D", "1/2", "1/2D", "1/1", "2/1"];
const QUARTERS: [f32; 18] =
    [0.0625, 1.0 / 12.0, 0.125, 1.0 / 6.0, 0.1875, 0.25, 1.0 / 3.0, 0.375, 0.5, 2.0 / 3.0, 0.75, 1.0, 4.0 / 3.0, 1.5, 2.0, 3.0, 4.0, 8.0];
pub const MAX_TIME_MS: f32 = 5000.0;

const P_STYLE: usize = 0;
const P_MODE: usize = 1;
const P_SYNC: usize = 2;
const P_TIME: usize = 3;
const P_NOTE: usize = 4;
const P_TIME_R: usize = 5;
const P_NOTE_R: usize = 6;
const P_FEEDBACK: usize = 7;
const P_MIX: usize = 8;
const P_SAT: usize = 9;
const P_WOBBLE: usize = 10;
const P_LOW: usize = 11;
const P_HIGH: usize = 12;
const P_DIFF: usize = 13;
const P_GROOVE: usize = 14;
const P_OUT: usize = 15;

static PARAMS: [ParamInfo; 16] = [
    choice("style", "Style", STYLES, 1),
    choice("mode", "Mode", MODES, 0),
    toggle("sync", "Sync", true),
    log("time", "Time", 1.0, MAX_TIME_MS, 375.0, Unit::Ms),
    choice("note", "Note", NOTES, 8),
    log("time_r", "Time R", 1.0, MAX_TIME_MS, 500.0, Unit::Ms),
    choice("note_r", "Note R", NOTES, 10),
    lin("feedback", "Feedback", 0.0, 110.0, 35.0, Unit::Percent),
    lin("mix", "Mix", 0.0, 100.0, 30.0, Unit::Percent),
    lin("saturation", "Saturation", 0.0, 100.0, 25.0, Unit::Percent),
    lin("wobble", "Wobble", 0.0, 100.0, 25.0, Unit::Percent),
    log("low_cut", "Low Cut", 20.0, 2000.0, 80.0, Unit::Hz),
    log("high_cut", "High Cut", 1000.0, 20000.0, 9000.0, Unit::Hz),
    lin("diffusion", "Diffusion", 0.0, 100.0, 0.0, Unit::Percent),
    lin("groove", "Groove", -50.0, 50.0, 0.0, Unit::Percent),
    lin("output", "Output", -24.0, 12.0, 0.0, Unit::Db),
];

pub static OPEN_ECHO_INFO: PluginInfo = PluginInfo {
    id: "open_echo",
    name: "OpenEcho",
    short_name: "OpenEcho",
    category: Category::Delay,
    params: &PARAMS,
    is_instrument: false,
    audiosuite: true,
};

pub struct Style {
    wow_ms: f32,
    wow_hz: f32,
    flutter_ms: f32,
    flutter_hz: f32,
    drive: f32,
    min_drive: f32,
    pub top_hz: f32,
    pub bottom_hz: f32,
    diffusion: f32,
    glide_ms: f32,
}

static STYLE_TABLE: [Style; 6] = [
    Style {
        wow_ms: 0.0,
        wow_hz: 0.5,
        flutter_ms: 0.0,
        flutter_hz: 6.0,
        drive: 1.5,
        min_drive: 0.25,
        top_hz: 20000.0,
        bottom_hz: 10.0,
        diffusion: 0.0,
        glide_ms: 60.0,
    },
    Style {
        wow_ms: 0.35,
        wow_hz: 0.6,
        flutter_ms: 0.05,
        flutter_hz: 7.5,
        drive: 3.0,
        min_drive: 0.4,
        top_hz: 14000.0,
        bottom_hz: 30.0,
        diffusion: 0.0,
        glide_ms: 200.0,
    },
    Style {
        wow_ms: 1.2,
        wow_hz: 0.9,
        flutter_ms: 0.18,
        flutter_hz: 9.0,
        drive: 4.0,
        min_drive: 0.6,
        top_hz: 6000.0,
        bottom_hz: 90.0,
        diffusion: 0.1,
        glide_ms: 300.0,
    },
    Style {
        wow_ms: 0.5,
        wow_hz: 1.4,
        flutter_ms: 0.0,
        flutter_hz: 6.0,
        drive: 2.5,
        min_drive: 0.5,
        top_hz: 3800.0,
        bottom_hz: 40.0,
        diffusion: 0.15,
        glide_ms: 120.0,
    },
    Style {
        wow_ms: 0.8,
        wow_hz: 1.1,
        flutter_ms: 0.3,
        flutter_hz: 11.0,
        drive: 6.0,
        min_drive: 1.0,
        top_hz: 3000.0,
        bottom_hz: 350.0,
        diffusion: 0.0,
        glide_ms: 150.0,
    },
    Style {
        wow_ms: 0.6,
        wow_hz: 0.4,
        flutter_ms: 0.0,
        flutter_hz: 6.0,
        drive: 1.5,
        min_drive: 0.25,
        top_hz: 9000.0,
        bottom_hz: 60.0,
        diffusion: 0.75,
        glide_ms: 200.0,
    },
];

pub fn style(i: usize) -> &'static Style {
    &STYLE_TABLE[i.min(STYLE_TABLE.len() - 1)]
}

pub fn note_ms(note: usize, tempo: f64) -> f32 {
    let q = QUARTERS[note.min(QUARTERS.len() - 1)];
    let tempo = if tempo.is_finite() && tempo > 1.0 { tempo as f32 } else { 120.0 };
    (q * 60_000.0 / tempo).clamp(1.0, MAX_TIME_MS)
}

#[inline]
fn saturate(x: f32, drive: f32) -> f32 {
    let d = drive.max(1.0e-3);
    let y = x * d;
    let t = y.clamp(-3.0, 3.0);
    (t * (27.0 + t * t) / (27.0 + 9.0 * t * t)) / d
}

#[derive(Default)]
struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn allocate(&mut self, len: usize) {
        self.buf = vec![0.0; len.max(1)];
        self.pos = 0;
    }

    #[inline]
    fn process(&mut self, x: f32, g: f32) -> f32 {
        let Some(slot) = self.buf.get_mut(self.pos) else { return x };
        let d = *slot;
        let v = x + g * d;
        *slot = flush(v);
        self.pos += 1;
        if self.pos >= self.buf.len() {
            self.pos = 0;
        }
        d - g * v
    }
}

struct Side {
    line: DelayLine,
    time: Smoother,
    hp: Biquad,
    lp: Biquad,
    diff: [Allpass; 3],
}

impl Default for Side {
    fn default() -> Side {
        Side { line: DelayLine::default(), time: Smoother::new(0.0), hp: Biquad::default(), lp: Biquad::default(), diff: Default::default() }
    }
}

pub struct OpenEcho {
    p: Params,
    sr: f32,
    ch: usize,
    tempo: f64,
    sides: [Side; 2],
    wow: [f32; 2],
    flutter: [f32; 2],
    drift: [f32; 2],
    rng: Rng,
    mix: Smoother,
    out: Smoother,
    fb: Smoother,
    scope: Arc<Scope>,
}

impl Default for OpenEcho {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenEcho {
    pub fn new() -> OpenEcho {
        let mut e = OpenEcho {
            p: Params::new(&PARAMS),
            sr: 48_000.0,
            ch: 2,
            tempo: 120.0,
            sides: Default::default(),
            wow: [0.0, 0.31],
            flutter: [0.0, 0.57],
            drift: [0.0; 2],
            rng: Rng::new(0x0EC4_0B0E),
            mix: Smoother::new(0.3),
            out: Smoother::new(1.0),
            fb: Smoother::new(0.35),
            scope: Scope::new(),
        };
        e.update();
        e.snap();
        e
    }

    fn snap(&mut self) {
        self.mix.snap();
        self.out.snap();
        self.fb.snap();
        self.sides.iter_mut().for_each(|s| s.time.snap());
    }

    pub fn times_ms(&self) -> (f32, f32) {
        times_ms(
            self.p.on(P_SYNC),
            self.p.v(P_TIME),
            self.p.choice(P_NOTE),
            self.p.v(P_TIME_R),
            self.p.choice(P_NOTE_R),
            self.p.choice(P_MODE),
            self.tempo,
        )
    }

    fn update(&mut self) {
        let sr = self.sr;
        let st = style(self.p.choice(P_STYLE));
        self.mix.set(self.p.v(P_MIX) / 100.0);
        self.out.set(crate::db_to_gain(self.p.v(P_OUT)));
        self.fb.set(self.p.v(P_FEEDBACK) / 100.0);
        let low = self.p.v(P_LOW).max(st.bottom_hz).min(sr * 0.2);
        let high = self.p.v(P_HIGH).min(st.top_hz).min(sr * 0.45);
        let (tl, tr) = self.times_ms();
        for (side, t) in self.sides.iter_mut().zip([tl, tr]) {
            side.hp.set(FilterType::HighPass, low, std::f32::consts::FRAC_1_SQRT_2, 0.0, sr);
            side.lp.set(FilterType::LowPass, high, std::f32::consts::FRAC_1_SQRT_2, 0.0, sr);
            side.time.set_time(st.glide_ms, sr);
            side.time.set(ms_to_samples(t, sr));
        }
    }
}

pub fn times_ms(sync: bool, time: f32, note: usize, time_r: f32, note_r: usize, mode: usize, tempo: f64) -> (f32, f32) {
    let pick = |t: f32, n: usize| if sync { note_ms(n, tempo) } else { t.clamp(1.0, MAX_TIME_MS) };
    let l = pick(time, note);
    let r = if mode == DUAL { pick(time_r, note_r) } else { l };
    (l, r)
}

pub fn echoes(mode: usize, feedback: f32, groove: f32, (tl, tr): (f32, f32), span: f32) -> Vec<(f32, f32, usize)> {
    let fb = (feedback / 100.0).clamp(0.0, 1.1).min(0.97);
    let mut out = Vec::new();
    let (l, r) = (tl * 0.001, tr * 0.001);
    let shift = 1.0 + groove / 100.0;
    let mut level = 1.0f32;
    for k in 0..256 {
        if level < 1.0e-3 {
            break;
        }
        match mode {
            PING_PONG => {
                let (j, side) = (k / 2, k % 2);
                let at = l * (1.0 + shift) * j as f32 + if side == 0 { l } else { l * (1.0 + shift) };
                if at > span {
                    break;
                }
                out.push((at, level, side));
            }
            _ => {
                let mut any = false;
                for (side, d) in [(0, l), (1, r)] {
                    let at = d * (k + 1) as f32;
                    if at <= span {
                        out.push((at, level, side));
                        any = true;
                    }
                }
                if !any {
                    break;
                }
            }
        }
        level *= fb;
    }
    out
}

impl Plugin for OpenEcho {
    param_plumbing!(OPEN_ECHO_INFO);

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = sane_sr(sample_rate);
        self.ch = sane_channels(channels);
        let sr = self.sr;
        let wobble = STYLE_TABLE.iter().map(|s| s.wow_ms + s.flutter_ms).fold(0.0, f32::max);
        for (k, side) in self.sides.iter_mut().enumerate() {
            side.line.allocate((ms_to_samples(MAX_TIME_MS * 1.5 + 2.0 * wobble, sr) + 8.0) as usize);
            for (ap, ms) in side.diff.iter_mut().zip([7.3, 11.9, 17.1]) {
                ap.allocate(ms_to_samples(ms * if k == 0 { 1.0 } else { 1.09 }, sr) as usize);
            }
        }
        self.mix.set_time(30.0, sr);
        self.out.set_time(30.0, sr);
        self.fb.set_time(30.0, sr);
        self.update();
        self.reset();
    }

    fn reset(&mut self) {
        for s in &mut self.sides {
            s.line.clear();
            s.hp.reset();
            s.lp.reset();
            for ap in &mut s.diff {
                ap.buf.iter_mut().for_each(|v| *v = 0.0);
            }
        }
        self.drift = [0.0; 2];
        self.snap();
    }

    fn set_transport(&mut self, transport: &Transport) {
        if let Some(mut d) = self.scope.try_write() {
            d.readout = transport.tempo as f32;
        }
        if transport.tempo.is_finite() && transport.tempo > 1.0 && (transport.tempo - self.tempo).abs() > 1.0e-6 {
            self.tempo = transport.tempo;
            if self.p.on(P_SYNC) {
                self.update();
            }
        }
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        let ch = self.ch.min(io.len());
        if ch == 0 {
            return;
        }
        let frames = frames_in(io, frames, ch);
        let sr = self.sr;
        let st = style(self.p.choice(P_STYLE));
        let mode = self.p.choice(P_MODE);
        let wobble = self.p.v(P_WOBBLE) / 100.0;
        let wow = ms_to_samples(st.wow_ms * wobble, sr);
        let flutter = ms_to_samples(st.flutter_ms * wobble, sr);
        let (wow_inc, flutter_inc) = (st.wow_hz / sr, st.flutter_hz / sr);
        let drive = st.min_drive + (st.drive - st.min_drive) * self.p.v(P_SAT) / 100.0;
        let g_diff = 0.65 * (st.diffusion + (1.0 - st.diffusion) * self.p.v(P_DIFF) / 100.0).min(1.0);
        let diffuse = g_diff > 0.01;
        let groove = 1.0 + self.p.v(P_GROOVE) / 100.0;
        let drift_c = 1.0 - (-1.0 / (0.25 * sr)).exp();
        for n in 0..frames {
            let xl = io.first().and_then(|b| b.get(n)).copied().filter(|v| v.is_finite()).unwrap_or(0.0);
            let xr = if ch > 1 { io.get(1).and_then(|b| b.get(n)).copied().filter(|v| v.is_finite()).unwrap_or(0.0) } else { xl };
            let fb = self.fb.next();
            let mut taps = [0.0f32; 2];
            for (k, side) in self.sides.iter_mut().enumerate() {
                self.wow[k] = (self.wow[k] + wow_inc).fract();
                self.flutter[k] = (self.flutter[k] + flutter_inc).fract();
                let (wf, fl) = (Lfo::sin_at(self.wow[k]), Lfo::sin_at(self.flutter[k]));
                self.drift[k] += drift_c * (self.rng.bipolar() - self.drift[k]);
                let mut d = side.time.next() + wow * (1.0 + wf + 0.6 * self.drift[k]) + flutter * (1.0 + fl);
                if mode == PING_PONG && k == 1 {
                    d *= groove;
                }
                taps[k] = side.line.read_cubic(d.max(2.0));
            }
            let (into_l, into_r) = match mode {
                PING_PONG => (0.5 * (xl + xr) + taps[1] * fb, taps[0] * fb),
                _ => (xl + taps[0] * fb, xr + taps[1] * fb),
            };
            for (side, x) in self.sides.iter_mut().zip([into_l, into_r]) {
                let mut v = side.hp.process_sample(x);
                v = side.lp.process_sample(v);
                if diffuse {
                    for ap in &mut side.diff {
                        v = ap.process(v, g_diff);
                    }
                }
                side.line.push(flush(saturate(v, drive)));
            }
            let m = self.mix.next();
            let g = self.out.next();
            for (c, b) in io.iter_mut().take(ch).enumerate() {
                let wet = if ch == 1 { 0.5 * (taps[0] + taps[1]) } else { taps[c % 2] };
                if let Some(x) = b.get_mut(n) {
                    let dry = if x.is_finite() { *x } else { 0.0 };
                    *x = (dry * (1.0 - m) + wet * m) * g;
                }
            }
        }
    }

    fn scope(&self) -> Option<Arc<Scope>> {
        Some(Arc::clone(&self.scope))
    }

    fn tail_samples(&self) -> usize {
        let (l, r) = self.times_ms();
        let t = l.max(r) * 0.001 * if self.p.choice(P_MODE) == PING_PONG { 2.0 } else { 1.0 };
        let fb = (self.p.v(P_FEEDBACK) / 100.0).clamp(0.0, 1.1);
        let repeats = if fb >= 0.98 {
            60.0
        } else if fb < 1.0e-3 {
            1.0
        } else {
            (1.0e-3f32.ln() / fb.ln()).clamp(1.0, 60.0)
        };
        ((t * (repeats + 1.0) + 0.2) * self.sr) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(e: &mut OpenEcho, secs: f32) -> Vec<Vec<f32>> {
        e.prepare(48_000.0, 512, 2);
        let total = (secs * 48_000.0) as usize;
        let mut out = vec![Vec::with_capacity(total), Vec::with_capacity(total)];
        let mut first = true;
        while out[0].len() < total {
            let mut io = vec![vec![0.0f32; 512], vec![0.0f32; 512]];
            if first {
                io[0][0] = 1.0;
                io[1][0] = 1.0;
                first = false;
            }
            e.process(&mut io, 512);
            assert!(io.iter().flatten().all(|v| v.is_finite()));
            for (o, b) in out.iter_mut().zip(io) {
                o.extend(b);
            }
        }
        out
    }

    fn clean(mode: usize) -> OpenEcho {
        let mut e = OpenEcho::new();
        for (k, v) in [
            ("style", 0.0),
            ("mode", mode as f32),
            ("sync", 0.0),
            ("time", 100.0),
            ("time_r", 150.0),
            ("mix", 100.0),
            ("wobble", 0.0),
            ("saturation", 0.0),
            ("low_cut", 20.0),
            ("high_cut", 20000.0),
            ("feedback", 50.0),
        ] {
            e.set_param(k, v);
        }
        e
    }

    fn peak_near(x: &[f32], at_ms: f32) -> f32 {
        let c = (at_ms * 48.0) as usize;
        x[c.saturating_sub(48)..c + 48].iter().fold(0.0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn single_repeats_at_the_time_and_falls_by_the_feedback() {
        let out = run(&mut clean(SINGLE), 0.5);
        let (a, b) = (peak_near(&out[0], 100.0), peak_near(&out[0], 200.0));
        assert!(a > 0.5, "first repeat {a}");
        assert!((b / a - 0.5).abs() < 0.12, "second repeat at half: {a} {b}");
        assert!(peak_near(&out[0], 50.0) < 1.0e-3, "nothing between");
    }

    #[test]
    fn dual_takes_its_own_right_time_and_ping_pong_alternates() {
        let out = run(&mut clean(DUAL), 0.5);
        assert!(peak_near(&out[1], 150.0) > 0.5 && peak_near(&out[1], 100.0) < 1.0e-3);
        let out = run(&mut clean(PING_PONG), 0.5);
        assert!(peak_near(&out[0], 100.0) > 0.5 && peak_near(&out[1], 100.0) < 1.0e-3, "left first");
        let (l1, r1, l2) = (peak_near(&out[0], 100.0), peak_near(&out[1], 200.0), peak_near(&out[0], 300.0));
        assert!((0.35..0.6).contains(&(r1 / l1)) && peak_near(&out[0], 200.0) < 1.0e-3, "then right: {l1} {r1}");
        assert!((0.35..0.6).contains(&(l2 / r1)), "then left again: {r1} {l2}");
    }

    #[test]
    fn synced_times_follow_the_tempo() {
        assert!((note_ms(11, 120.0) - 500.0).abs() < 1.0e-3, "a quarter at 120");
        assert!((note_ms(10, 120.0) - 375.0).abs() < 1.0e-3, "a dotted eighth");
        assert!((note_ms(6, 90.0) - 222.222).abs() < 0.01, "an eighth triplet at 90");
        let mut e = OpenEcho::new();
        e.set_param("note", 11.0);
        e.set_transport(&Transport { tempo: 100.0, ..Default::default() });
        assert!((e.times_ms().0 - 600.0).abs() < 1.0e-3);
    }

    #[test]
    fn feedback_over_one_hundred_stays_bounded_in_every_style() {
        for s in 0..STYLES.len() {
            let mut e = clean(SINGLE);
            e.set_param("style", s as f32);
            e.set_param("feedback", 110.0);
            e.set_param("saturation", 0.0);
            e.set_param("wobble", 100.0);
            e.prepare(48_000.0, 512, 2);
            let mut peak = 0.0f32;
            for blk in 0..400 {
                let mut io: Vec<Vec<f32>> =
                    (0..2).map(|_| (0..512).map(|i| if blk < 4 { ((i as f32) * 0.07).sin() * 0.8 } else { 0.0 }).collect()).collect();
                e.process(&mut io, 512);
                peak = io.iter().flatten().fold(peak, |m, v| m.max(v.abs()));
                assert!(io.iter().flatten().all(|v| v.is_finite()));
            }
            assert!(peak < 8.0, "style {s}: {peak}");
        }
    }
}
