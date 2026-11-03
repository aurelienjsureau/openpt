use crate::params::{Params, choice, lin, log, param_plumbing, toggle};
use crate::scope::{Column, Scope};
use crate::{Category, ParamInfo, Plugin, PluginInfo, Unit};
use std::collections::VecDeque;
use std::sync::Arc;

pub const STYLES: &[&str] = &["Transparent", "Allround", "Aggressive"];
pub const TRANSPARENT: usize = 0;
pub const AGGRESSIVE: usize = 2;

const P_GAIN: usize = 0;
const P_CEILING: usize = 1;
const P_STYLE: usize = 2;
const P_RELEASE: usize = 3;
const P_LOOKAHEAD: usize = 4;
const P_TRUE_PEAK: usize = 5;
const P_LINK: usize = 6;

static PARAMS: [ParamInfo; 7] = [
    lin("gain", "Gain", 0.0, 30.0, 0.0, Unit::Db),
    lin("ceiling", "Output Level", -30.0, 0.0, -1.0, Unit::Db),
    choice("style", "Style", STYLES, 1),
    log("release", "Release", 1.0, 1000.0, 80.0, Unit::Ms),
    lin("lookahead", "Lookahead", 0.5, 10.0, 3.0, Unit::Ms),
    toggle("true_peak", "True Peak", true),
    lin("link", "Stereo Link", 0.0, 100.0, 100.0, Unit::Percent),
];

pub static OPEN_L_INFO: PluginInfo = PluginInfo {
    id: "open_l",
    name: "OpenL",
    short_name: "OpenL",
    category: Category::Dynamics,
    params: &PARAMS,
    is_instrument: false,
    audiosuite: true,
};

const MAX_LOOKAHEAD_MS: f32 = 10.0;
const TP_SAFETY: f32 = 0.995;

fn release_scale(style: usize, gr_db: f32) -> f32 {
    match style {
        TRANSPARENT => 1.0 + (gr_db / 6.0).clamp(0.0, 3.0),
        AGGRESSIVE => 0.35,
        _ => 1.0 + (gr_db / 12.0).clamp(0.0, 1.0),
    }
}

#[inline]
fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + y1
}

struct Lane {
    hist: [f32; 4],
    hold: VecDeque<(u64, f32)>,
    env: f32,
    sum: f64,
    ring: Vec<f32>,
}

impl Lane {
    fn new(len: usize) -> Lane {
        Lane { hist: [0.0; 4], hold: VecDeque::with_capacity(len + 2), env: 1.0, sum: len as f64, ring: vec![1.0; len.max(1)] }
    }
}

pub struct OpenL {
    p: Params,
    sr: f32,
    ch: usize,
    lanes: [Lane; 2],
    delay: [Vec<f32>; 2],
    dpos: usize,
    win: usize,
    t: u64,
    gr_now: f32,
    col_len: usize,
    col_n: usize,
    scope: Arc<Scope>,
}

impl Default for OpenL {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenL {
    pub fn new() -> OpenL {
        OpenL {
            p: Params::new(&PARAMS),
            sr: 48_000.0,
            ch: 2,
            lanes: [Lane::new(1), Lane::new(1)],
            delay: [vec![0.0; 4], vec![0.0; 4]],
            dpos: 0,
            win: 1,
            t: 0,
            gr_now: 0.0,
            col_len: 480,
            col_n: 0,
            scope: Scope::new(),
        }
    }

    fn update(&mut self) {}

    fn window(&self) -> usize {
        ((self.p.v(P_LOOKAHEAD).clamp(0.5, MAX_LOOKAHEAD_MS) * 0.001 * self.sr).round() as usize).max(1) + 2
    }

    fn rebuild(&mut self) {
        self.win = self.window();
        let w = self.win;
        self.lanes = [Lane::new(w), Lane::new(w)];
        let dl = (MAX_LOOKAHEAD_MS * 0.001 * self.sr) as usize + 8;
        self.delay = [vec![0.0; dl], vec![0.0; dl]];
        self.dpos = 0;
    }

    fn run(&mut self, io: &mut [Vec<f32>], frames: usize) {
        let nch = io.len().min(self.ch).min(2);
        if nch == 0 {
            return;
        }
        if self.window() != self.win {
            self.rebuild();
        }
        let n = frames.min(io[..nch].iter().map(Vec::len).min().unwrap_or(0));
        let drive = crate::db_to_gain(self.p.v(P_GAIN));
        let ceiling = crate::db_to_gain(self.p.v(P_CEILING));
        let style = self.p.choice(P_STYLE);
        let tp = self.p.on(P_TRUE_PEAK);
        let link = self.p.v(P_LINK) * 0.01;
        let rel_ms = self.p.v(P_RELEASE);
        let sr = self.sr;
        let w = self.win;
        let lat = w - 1;
        let dlen = self.delay[0].len();
        let mut col_in = 0.0f32;
        let mut col_out = 0.0f32;
        let mut col_gr = 0.0f32;
        for i in 0..n {
            let mut need = [1.0f32; 2];
            for c in 0..nch {
                let x = io[c][i] * drive;
                col_in = col_in.max(x.abs());
                let lane = &mut self.lanes[c];
                lane.hist = [lane.hist[1], lane.hist[2], lane.hist[3], x];
                let mut peak = x.abs();
                if tp {
                    let [a, b, cc, d] = lane.hist;
                    for t in [0.25f32, 0.5, 0.75] {
                        peak = peak.max(hermite(a, b, cc, d, t).abs());
                    }
                    peak = peak.max(b.abs()).max(cc.abs());
                }
                let lim = if tp { ceiling * TP_SAFETY } else { ceiling };
                need[c] = if peak > lim { lim / peak } else { 1.0 };
            }
            if nch == 1 {
                need[1] = need[0];
            }
            let linked = need[0].min(need[1]);
            let mut gr_max = 0.0f32;
            for c in 0..nch {
                let req = linked * link + need[c] * (1.0 - link);
                let lane = &mut self.lanes[c];
                while lane.hold.back().is_some_and(|&(_, v)| v >= req) {
                    lane.hold.pop_back();
                }
                lane.hold.push_back((self.t, req));
                while lane.hold.front().is_some_and(|&(at, _)| at + w as u64 <= self.t) {
                    lane.hold.pop_front();
                }
                let held = lane.hold.front().map_or(1.0, |&(_, v)| v);
                if held < lane.env {
                    lane.env = held;
                } else {
                    let gr_db = -crate::gain_to_db(lane.env.max(1.0e-6));
                    let ms = rel_ms * release_scale(style, gr_db);
                    let k = 1.0 - (-1000.0 / (ms * sr)).exp();
                    lane.env += k * (held - lane.env);
                }
                let slot = (self.t % w as u64) as usize;
                lane.sum += f64::from(lane.env) - f64::from(lane.ring[slot]);
                lane.ring[slot] = lane.env;
                let g = (lane.sum / w as f64) as f32;
                let line = &mut self.delay[c];
                line[self.dpos] = io[c][i] * drive;
                let dry = line[(self.dpos + dlen - lat) % dlen];
                let mut y = dry * g.min(1.0);
                if style == AGGRESSIVE && y.abs() > ceiling * 0.9 {
                    let k = ceiling * 0.9;
                    let over = (y.abs() - k) / (ceiling - k);
                    y = y.signum() * (k + (ceiling - k) * over.tanh());
                }
                y = y.clamp(-ceiling, ceiling);
                col_out = col_out.max(y.abs());
                gr_max = gr_max.max(-crate::gain_to_db(g.clamp(1.0e-6, 1.0)));
                io[c][i] = y;
            }
            self.dpos = (self.dpos + 1) % dlen;
            self.t += 1;
            self.gr_now = gr_max;
            col_gr = col_gr.max(gr_max);
            self.col_n += 1;
            if self.col_n >= self.col_len {
                self.col_n = 0;
                let col = Column {
                    input: crate::gain_to_db(col_in.max(1.0e-7)),
                    output: crate::gain_to_db(col_out.max(1.0e-7)),
                    gr: col_gr,
                    aux: crate::gain_to_db(ceiling),
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
            }
        }
    }
}

impl Plugin for OpenL {
    param_plumbing!(OPEN_L_INFO);

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = if sample_rate.is_finite() && sample_rate > 0.0 { sample_rate.clamp(8_000.0, 384_000.0) } else { 48_000.0 };
        self.ch = channels.clamp(1, 2);
        self.t = 0;
        self.rebuild();
        self.col_len = ((0.01 * self.sr) as usize).max(1);
        self.col_n = 0;
    }

    fn reset(&mut self) {
        self.t = 0;
        self.rebuild();
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        self.run(io, frames);
    }

    fn latency(&self) -> usize {
        self.window() - 1
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

    fn drive(l: &mut OpenL, f: f32, amp: f32, blocks: usize) -> (f32, Vec<f32>) {
        let mut peak = 0.0f32;
        let mut last = Vec::new();
        for blk in 0..blocks {
            let x: Vec<f32> = (0..512).map(|i| amp * (std::f32::consts::TAU * f * (blk * 512 + i) as f32 / 48_000.0).sin()).collect();
            let mut io = vec![x.clone(), x];
            l.process(&mut io, 512);
            assert!(io[0].iter().all(|v| v.is_finite()));
            if blk > 4 {
                peak = io[0].iter().fold(peak, |m, v| m.max(v.abs()));
            }
            last = io.swap_remove(0);
        }
        (peak, last)
    }

    #[test]
    fn the_ceiling_holds_for_every_style() {
        for style in 0..STYLES.len() {
            let mut l = OpenL::new();
            l.set_param("style", style as f32);
            l.set_param("gain", 18.0);
            l.set_param("ceiling", -1.0);
            l.prepare(48_000.0, 512, 2);
            let (peak, _) = drive(&mut l, 997.0, 0.5, 80);
            assert!(peak <= crate::db_to_gain(-1.0) + 1.0e-6, "style {style}: {peak}");
            assert!(l.gain_reduction_db() > 10.0, "style {style}: gr {}", l.gain_reduction_db());
        }
    }

    #[test]
    fn quiet_material_passes_untouched_after_the_latency() {
        let mut l = OpenL::new();
        l.prepare(48_000.0, 512, 1);
        let lat = l.latency();
        let x: Vec<f32> = (0..4096).map(|i| 0.1 * (i as f32 * 0.01).sin()).collect();
        let mut io = vec![x.clone()];
        l.process(&mut io, 4096);
        let err = (lat..4096).map(|i| (io[0][i] - x[i - lat]).abs()).fold(0.0, f32::max);
        assert!(err < 1.0e-6, "{err}");
    }

    #[test]
    fn a_lone_transient_is_caught_before_it_arrives() {
        let mut l = OpenL::new();
        l.set_param("ceiling", -6.0);
        l.set_param("true_peak", 0.0);
        l.prepare(48_000.0, 512, 1);
        let mut x = vec![0.0f32; 2048];
        x[1000] = 1.0;
        let mut io = vec![x];
        l.process(&mut io, 2048);
        let peak = io[0].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - crate::db_to_gain(-6.0)).abs() < 0.02, "{peak}");
    }
}
