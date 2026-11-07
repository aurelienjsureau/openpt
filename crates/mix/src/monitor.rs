#![allow(clippy::needless_range_loop)]

use crate::hrtf::{self, HrirSet};
use openpt_model::{AtmosMonitor, AtmosSettings, BinauralDistance, ChannelFormat, Route, Session, Speaker, TrackKind};

const MAX_CHUNK: usize = 4096;
const REFLECTION_TAPS: usize = 128;
const WALLS: [(usize, f32); 6] = [(0, -2.5), (0, 2.5), (1, -3.0), (1, 3.0), (2, -1.2), (2, 1.8)];
const SPEED_OF_SOUND: f32 = 343.0;
const WALL_LOSS: f32 = 0.6;
const WALL_CUTOFF_HZ: f32 = 7_000.0;
const BUS_DIRECTIONS: [(f32, f32); 9] =
    [(0.0, 0.0), (0.0, 30.0), (0.0, -30.0), (0.0, 90.0), (0.0, -90.0), (0.0, 150.0), (0.0, -150.0), (45.0, 90.0), (45.0, -90.0)];
const LATE_LINES_MS: [f32; 4] = [23.1, 29.7, 37.3, 43.9];
const LATE_T60_S: f32 = 0.35;
const LATE_CUTOFF_HZ: f32 = 6_000.0;

struct Ear {
    left: Vec<f32>,
    right: Vec<f32>,
}

struct Reflection {
    delay: usize,
    gain: f32,
    bus: usize,
}

struct Channel {
    source: usize,
    slot: usize,
    bed: bool,
    lfe: bool,
    plain: Option<(f32, f32)>,
    ear: Option<Ear>,
    history: Vec<f32>,
    ring: Vec<f32>,
    reflections: Vec<Reflection>,
    late: f32,
}

struct Bus {
    ear: Ear,
    history: Vec<f32>,
    input: Vec<f32>,
    state: f32,
}

struct Late {
    lines: [Vec<f32>; 4],
    pos: [usize; 4],
    feedback: [f32; 4],
    damped: [f32; 4],
    keep: f32,
}

impl Late {
    fn new(rate: u32) -> Late {
        let rate = rate.max(1) as f32;
        let lines = LATE_LINES_MS.map(|ms| vec![0.0f32; ((ms * 0.001 * rate) as usize).max(1)]);
        let feedback = LATE_LINES_MS.map(|ms| 10f32.powf(-3.0 * ms * 0.001 / LATE_T60_S));
        Late { lines, pos: [0; 4], feedback, damped: [0.0; 4], keep: (-std::f32::consts::TAU * LATE_CUTOFF_HZ / rate).exp() }
    }

    fn tick(&mut self, input: f32) -> (f32, f32) {
        let mut read = [0.0f32; 4];
        for i in 0..4 {
            read[i] = self.lines[i][self.pos[i]];
            self.damped[i] = read[i] + self.keep * (self.damped[i] - read[i]);
        }
        let v: [f32; 4] = std::array::from_fn(|i| self.damped[i] * self.feedback[i]);
        let half = 0.5 * v.iter().sum::<f32>();
        for (i, sign) in [1.0f32, -1.0, 1.0, -1.0].into_iter().enumerate() {
            self.lines[i][self.pos[i]] = v[i] - half + input * sign;
            self.pos[i] = (self.pos[i] + 1) % self.lines[i].len();
        }
        (0.5 * (read[0] - read[1] + read[2] - read[3]), 0.5 * (read[0] + read[1] - read[2] - read[3]))
    }
}

#[derive(PartialEq)]
struct Key {
    monitor: AtmosMonitor,
    fmt: ChannelFormat,
    rate: u32,
    trims: (i32, i32, i32),
    distance: [BinauralDistance; openpt_model::BINAURAL_CHANNELS],
    objects: u8,
    hrtf: u64,
}

pub struct Monitor {
    key: Key,
    mode: AtmosMonitor,
    speakers: Vec<Speaker>,
    channels: Vec<Channel>,
    buses: Vec<Bus>,
    late: Option<Late>,
    late_in: Vec<f32>,
    write: usize,
    wall_keep: f32,
    matrix: Vec<[f32; 6]>,
    out: Vec<Vec<f32>>,
    scratch: Vec<f32>,
    input: Vec<f32>,
    gain: f32,
}

pub fn split_modes(s: &Session) -> u8 {
    let fmt = s.main_format();
    if s.atmos.monitor != AtmosMonitor::Binaural || !Monitor::wanted(&s.atmos, fmt) {
        return 0;
    }
    let objects = s
        .tracks
        .iter()
        .filter(|t| t.kind != TrackKind::Master && !t.inactive && t.mixer.output == Route::Main && s.edit.is_object(t.id))
        .fold(0u8, |mask, t| mask | (1 << t.mixer.binaural_distance.index()));
    let bed = fmt
        .speakers()
        .iter()
        .enumerate()
        .filter(|(_, sp)| !sp.is_lfe())
        .fold(0u8, |mask, (c, _)| mask | (1 << s.atmos.binaural_distance.get(c).copied().unwrap_or_default().index()));
    if objects == bed && bed.count_ones() == 1 { 0 } else { objects }
}

fn db(v: f32) -> f32 {
    10f32.powf(v / 20.0)
}

fn profile(d: BinauralDistance) -> Option<(f32, f32, f32, f32)> {
    match d {
        BinauralDistance::Off => None,
        BinauralDistance::Near => Some((0.5, 1.0, 0.4, 0.04)),
        BinauralDistance::Mid => Some((1.5, 0.89, 1.0, 0.1)),
        BinauralDistance::Far => Some((2.2, 0.71, 1.8, 0.2)),
    }
}

fn ear_for(set: &HrirSet, elevation: f32, azimuth: f32, rate: u32, gain: f32, max_taps: Option<usize>) -> Option<Ear> {
    let h = set.at(elevation, azimuth)?;
    let at_rate = |ir: &[f32]| -> Vec<f32> {
        let mut out = if rate == set.rate || rate == 0 {
            ir.to_vec()
        } else {
            let scale = rate as f32 / set.rate as f32;
            openpt_dsp::offline::resample(&[ir.to_vec()], set.rate, rate)
                .into_iter()
                .next()
                .unwrap_or_default()
                .into_iter()
                .map(|x| x / scale)
                .collect()
        };
        if let Some(m) = max_taps {
            out.truncate((m as f32 * rate.max(1) as f32 / set.rate.max(1) as f32) as usize);
        }
        out.iter_mut().for_each(|v| *v *= gain);
        out
    };
    Some(Ear { left: at_rate(&h.left), right: at_rate(&h.right) })
}

fn angle_between(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

fn bus_for(elevation: f32, azimuth: f32) -> usize {
    BUS_DIRECTIONS
        .iter()
        .enumerate()
        .filter(|(_, (el, _))| (*el > 25.0) == (elevation > 25.0))
        .min_by(|(_, a), (_, b)| angle_between(a.1, azimuth).total_cmp(&angle_between(b.1, azimuth)))
        .map_or(0, |(i, _)| i)
}

fn reflections_for(sp: Speaker, distance: f32, level: f32, rate: u32) -> Vec<Reflection> {
    let (el, az) = (sp.elevation().to_radians(), sp.azimuth().to_radians());
    let source = [distance * el.cos() * az.sin(), distance * el.cos() * az.cos(), distance * el.sin()];
    WALLS
        .iter()
        .map(|(axis, wall)| {
            let mut image = source;
            image[*axis] = 2.0 * wall - source[*axis];
            let reach = (image[0] * image[0] + image[1] * image[1] + image[2] * image[2]).sqrt().max(0.01);
            let delay = (((reach - distance).max(0.0) / SPEED_OF_SOUND * rate as f32).round() as usize).max(1);
            let elevation = (image[2] / reach).asin().to_degrees();
            let azimuth = image[0].atan2(image[1]).to_degrees();
            Reflection { delay, gain: level * WALL_LOSS * 1.5 / reach, bus: bus_for(elevation, azimuth) }
        })
        .collect()
}

fn lr(g: f32, s: f32) -> [f32; 2] {
    let h = std::f32::consts::FRAC_1_SQRT_2;
    if s < 0.0 {
        [g, 0.0]
    } else if s > 0.0 {
        [0.0, g]
    } else {
        [g * h, g * h]
    }
}

fn side(sp: Speaker) -> f32 {
    let a = sp.azimuth();
    if a.abs() < 1.0 || (a.abs() - 180.0).abs() < 1.0 { 0.0 } else { a.signum() }
}

fn downmix_row(sp: Speaker, to51: bool, surround: f32, height: f32) -> [f32; 6] {
    use Speaker::*;
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let mut row = [0.0f32; 6];
    if to51 {
        match sp {
            L | Lw | Lc => row[0] = 1.0,
            R | Rw | Rc => row[1] = 1.0,
            C => row[2] = 1.0,
            Lfe => row[3] = 1.0,
            Ls | Lss | Lrs => row[4] = 1.0,
            Rs | Rss | Rrs => row[5] = 1.0,
            Cs => {
                row[4] = h;
                row[5] = h;
            }
            Ltf => row[0] = height,
            Rtf => row[1] = height,
            Ltr => row[4] = height,
            Rtr => row[5] = height,
            Ltm => {
                row[0] = height * h;
                row[4] = height * h;
            }
            Rtm => {
                row[1] = height * h;
                row[5] = height * h;
            }
        }
        return row;
    }
    let [l, r] = match sp {
        Lfe => [0.0, 0.0],
        C => [h, h],
        L | R | Lw | Rw | Lc | Rc => lr(1.0, side(sp)),
        Ls | Rs | Lss | Rss | Lrs | Rrs | Cs => lr(surround, side(sp)),
        _ => lr(height, side(sp)),
    };
    row[0] = l;
    row[1] = r;
    row
}

fn convolve(history: &mut [f32], src: &[f32], ear: &Ear, l: &mut [f32], r: &mut [f32], buf: &mut Vec<f32>) {
    if src.iter().all(|v| *v == 0.0) && history.iter().all(|v| *v == 0.0) {
        return;
    }
    let taps = history.len();
    let frames = src.len();
    buf.clear();
    buf.extend_from_slice(history);
    buf.extend_from_slice(src);
    for (ir, out) in [(&ear.left, l), (&ear.right, r)] {
        for (k, g) in ir.iter().enumerate().take(taps) {
            let start = taps - k;
            for (o, x) in out.iter_mut().zip(&buf[start..start + frames]) {
                *o += g * x;
            }
        }
    }
    history.copy_from_slice(&buf[frames..]);
}

impl Monitor {
    pub fn wanted(settings: &AtmosSettings, fmt: ChannelFormat) -> bool {
        settings.monitor != AtmosMonitor::Speakers && fmt.channels() > 2 && !fmt.speakers().is_empty()
    }

    pub fn new(settings: &AtmosSettings, fmt: ChannelFormat, rate: u32) -> Monitor {
        Monitor::with_objects(settings, fmt, rate, 0)
    }

    pub fn with_objects(settings: &AtmosSettings, fmt: ChannelFormat, rate: u32, objects: u8) -> Monitor {
        let speakers = fmt.speakers().to_vec();
        let to51 = settings.monitor == AtmosMonitor::Surround51;
        let (surround, height) = (db(settings.surround_trim_db), db(settings.height_trim_db));
        let binaural = settings.monitor == AtmosMonitor::Binaural;
        let key = Monitor::key_of(settings, fmt, rate, objects);
        let set = hrtf::current();
        let mut channels = Vec::new();
        let mut buses = Vec::new();
        if binaural {
            let delays_for = |sp: Speaker, d: BinauralDistance| {
                profile(d).filter(|_| !sp.is_lfe()).map(|(distance, _, level, _)| reflections_for(sp, distance, level, rate))
            };
            let bed_of = |c: usize| settings.binaural_distance.get(c).copied().unwrap_or_default();
            let wanted = |c: usize, sp: Speaker, d: BinauralDistance| bed_of(c) == d || (!sp.is_lfe() && objects & (1 << d.index()) != 0);
            let longest = speakers
                .iter()
                .enumerate()
                .flat_map(|(c, sp)| BinauralDistance::ALL.into_iter().filter(move |d| wanted(c, *sp, *d)).filter_map(move |d| delays_for(*sp, d)))
                .flatten()
                .map(|r| r.delay)
                .max()
                .unwrap_or(0);
            for (c, sp) in speakers.iter().enumerate() {
                for d in BinauralDistance::ALL.into_iter().filter(|d| wanted(c, *sp, *d)) {
                    let (source, slot, bed, lfe) = (c, d.index(), bed_of(c) == d, sp.is_lfe());
                    let channel = match profile(d).filter(|_| !lfe) {
                        None => {
                            let [l, r] = if lfe { [1.0, 1.0] } else { lr(1.0, side(*sp)) };
                            Channel {
                                source,
                                slot,
                                bed,
                                lfe,
                                plain: Some((l, r)),
                                ear: None,
                                history: Vec::new(),
                                ring: Vec::new(),
                                reflections: Vec::new(),
                                late: 0.0,
                            }
                        }
                        Some((distance, direct, level, late)) => {
                            let ear = ear_for(&set, sp.elevation(), sp.azimuth(), rate, direct, None);
                            let taps = ear.as_ref().map_or(1, |e| e.left.len().max(e.right.len())).max(1);
                            Channel {
                                source,
                                slot,
                                bed,
                                lfe,
                                plain: None,
                                ear,
                                history: vec![0.0; taps],
                                ring: vec![0.0; longest + MAX_CHUNK + 1],
                                reflections: reflections_for(*sp, distance, level, rate),
                                late,
                            }
                        }
                    };
                    channels.push(channel);
                }
            }
            for (elevation, azimuth) in BUS_DIRECTIONS {
                if let Some(ear) = ear_for(&set, elevation, azimuth, rate, 1.0, Some(REFLECTION_TAPS)) {
                    let taps = ear.left.len().max(ear.right.len()).max(1);
                    buses.push(Bus { ear, history: vec![0.0; taps], input: Vec::new(), state: 0.0 });
                }
            }
        }
        let late = channels.iter().any(|c| c.late > 0.0).then(|| Late::new(rate));
        Monitor {
            key,
            mode: settings.monitor,
            matrix: speakers.iter().map(|s| downmix_row(*s, to51, surround, height)).collect(),
            channels,
            buses,
            late,
            late_in: Vec::new(),
            write: 0,
            wall_keep: (-std::f32::consts::TAU * WALL_CUTOFF_HZ / rate.max(1) as f32).exp(),
            speakers,
            out: vec![Vec::new(); 6],
            scratch: Vec::new(),
            input: Vec::new(),
            gain: db(settings.binaural_gain_db),
        }
    }

    fn key_of(settings: &AtmosSettings, fmt: ChannelFormat, rate: u32, objects: u8) -> Key {
        let k = |v: f32| (v * 100.0).round() as i32;
        Key {
            monitor: settings.monitor,
            fmt,
            rate,
            trims: (k(settings.surround_trim_db), k(settings.height_trim_db), k(settings.binaural_gain_db)),
            distance: settings.binaural_distance,
            objects,
            hrtf: if settings.monitor == AtmosMonitor::Binaural { hrtf::generation() } else { 0 },
        }
    }

    pub fn matches(&self, settings: &AtmosSettings, fmt: ChannelFormat, rate: u32, objects: u8) -> bool {
        self.key == Monitor::key_of(settings, fmt, rate, objects)
    }

    pub fn outputs(&self) -> usize {
        match self.mode {
            AtmosMonitor::Surround51 => 6,
            _ => 2,
        }
    }

    pub fn process(&mut self, main: &mut [Vec<f32>], frames: usize) {
        self.run(main, None, frames);
    }

    pub fn process_objects(&mut self, main: &mut [Vec<f32>], feeds: &[Vec<Vec<f32>>; 4], frames: usize) {
        self.run(main, Some(feeds), frames);
    }

    fn run(&mut self, main: &mut [Vec<f32>], feeds: Option<&[Vec<Vec<f32>>; 4]>, frames: usize) {
        let n_out = self.outputs();
        for o in self.out.iter_mut().take(n_out) {
            o.clear();
            o.resize(frames, 0.0);
        }
        if self.mode == AtmosMonitor::Binaural {
            let mut start = 0;
            while start < frames {
                let len = (frames - start).min(MAX_CHUNK);
                self.binaural(main, feeds, start, len);
                start += len;
            }
        } else {
            for (c, row) in self.matrix.iter().enumerate() {
                let Some(src) = main.get(c) else { continue };
                for (o, g) in self.out.iter_mut().zip(row).take(n_out) {
                    if *g == 0.0 {
                        continue;
                    }
                    for (d, s) in o.iter_mut().zip(src.iter().take(frames)) {
                        *d += s * g;
                    }
                }
            }
        }
        for (c, ch) in main.iter_mut().enumerate().take(self.speakers.len()) {
            let src = self.out.get(c).filter(|_| c < n_out);
            for (i, x) in ch.iter_mut().take(frames).enumerate() {
                *x = src.and_then(|s| s.get(i)).copied().unwrap_or(0.0);
            }
        }
    }

    fn binaural(&mut self, main: &[Vec<f32>], feeds: Option<&[Vec<Vec<f32>>; 4]>, start: usize, len: usize) {
        let [l, r, ..] = &mut self.out[..] else { return };
        let (Some(l), Some(r)) = (l.get_mut(start..start + len), r.get_mut(start..start + len)) else { return };
        for bus in &mut self.buses {
            bus.input.clear();
            bus.input.resize(len, 0.0);
        }
        self.late_in.clear();
        self.late_in.resize(len, 0.0);
        let write = self.write;
        for ch in &mut self.channels {
            let Some(base) = main.get(ch.source).and_then(|s| s.get(start..start + len)) else { continue };
            let src: &[f32] = match feeds {
                None => base,
                Some(feeds) => {
                    self.input.clear();
                    if ch.bed {
                        self.input.extend_from_slice(base);
                    } else {
                        self.input.resize(len, 0.0);
                    }
                    if !ch.lfe {
                        for (slot, feed) in feeds.iter().enumerate() {
                            let Some(f) = feed.get(ch.source).and_then(|c| c.get(start..start + len)) else { continue };
                            let sign = match (slot == ch.slot, ch.bed) {
                                (true, false) => 1.0,
                                (false, true) => -1.0,
                                _ => continue,
                            };
                            for (o, x) in self.input.iter_mut().zip(f) {
                                *o += sign * x;
                            }
                        }
                    }
                    &self.input
                }
            };
            if let Some((gl, gr)) = ch.plain {
                for (i, x) in src.iter().enumerate() {
                    l[i] += x * gl;
                    r[i] += x * gr;
                }
                continue;
            }
            if let Some(ear) = &ch.ear {
                convolve(&mut ch.history, src, ear, l, r, &mut self.scratch);
            }
            let n = ch.ring.len();
            if n == 0 {
                continue;
            }
            for (i, x) in src.iter().enumerate() {
                ch.ring[(write + i) % n] = *x;
            }
            for t in &ch.reflections {
                let Some(bus) = self.buses.get_mut(t.bus) else { continue };
                for (i, o) in bus.input.iter_mut().enumerate() {
                    *o += t.gain * ch.ring[(write + i + n - t.delay) % n];
                }
            }
            for (o, x) in self.late_in.iter_mut().zip(src) {
                *o += ch.late * x;
            }
        }
        if let Some(n) = self.channels.iter().map(|c| c.ring.len()).find(|n| *n > 0) {
            self.write = (write + len) % n;
        }
        let keep = self.wall_keep;
        for bus in &mut self.buses {
            for x in &mut bus.input {
                bus.state = *x + keep * (bus.state - *x);
                *x = bus.state;
            }
            convolve(&mut bus.history, &bus.input, &bus.ear, l, r, &mut self.scratch);
        }
        if let Some(late) = &mut self.late {
            for (i, x) in self.late_in.iter().enumerate() {
                let (a, b) = late.tick(*x);
                l[i] += a;
                r[i] += b;
            }
        }
        for v in l.iter_mut().chain(r.iter_mut()) {
            *v *= self.gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse_on(fmt: ChannelFormat, sp: Speaker, n: usize) -> Vec<Vec<f32>> {
        let mut main = vec![vec![0.0f32; n]; fmt.channels()];
        let c = fmt.speakers().iter().position(|s| *s == sp).unwrap();
        main[c][0] = 1.0;
        main
    }

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|v| v * v).sum()
    }

    fn with_distance(d: BinauralDistance) -> AtmosSettings {
        AtmosSettings { monitor: AtmosMonitor::Binaural, binaural_distance: [d; openpt_model::BINAURAL_CHANNELS], ..AtmosSettings::default() }
    }

    fn band_db(x: &[f32], rate: f32, lo: f32, hi: f32) -> f32 {
        let (mut sum, mut count, mut f) = (0.0f32, 0.0f32, lo);
        while f < hi {
            let w = std::f32::consts::TAU * f / rate;
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (n, v) in x.iter().enumerate() {
                re += v * (w * n as f32).cos();
                im += v * (w * n as f32).sin();
            }
            sum += re * re + im * im;
            count += 1.0;
            f += 100.0;
        }
        10.0 * (sum / count.max(1.0)).log10()
    }

    #[test]
    fn binaural_puts_each_speaker_on_its_side() {
        let s = with_distance(BinauralDistance::Near);
        for (sp, right_louder) in [(Speaker::R, true), (Speaker::Lss, false), (Speaker::Rtr, true), (Speaker::Ltf, false)] {
            let mut m = Monitor::new(&s, ChannelFormat::Atmos714, 48_000);
            let mut main = impulse_on(ChannelFormat::Atmos714, sp, 512);
            m.process(&mut main, 512);
            let (l, r) = (energy(&main[0]), energy(&main[1]));
            assert!(l > 0.0 && r > 0.0, "{sp:?}");
            assert_eq!(r > l * 2.0, right_louder, "{sp:?}: L {l} R {r}");
            assert!(main[2..].iter().all(|c| c.iter().all(|v| *v == 0.0)), "only the first two channels carry the render");
        }
    }

    #[test]
    fn binaural_carries_over_between_blocks() {
        let s = AtmosSettings { monitor: AtmosMonitor::Binaural, ..AtmosSettings::default() };
        let mut whole = Monitor::new(&s, ChannelFormat::Atmos714, 44_100);
        let mut split = Monitor::new(&s, ChannelFormat::Atmos714, 44_100);
        let mut a = impulse_on(ChannelFormat::Atmos714, Speaker::C, 9000);
        whole.process(&mut a, 9000);
        let mut b1 = impulse_on(ChannelFormat::Atmos714, Speaker::C, 64);
        split.process(&mut b1, 64);
        let mut b2 = vec![vec![0.0f32; 8936]; 12];
        split.process(&mut b2, 8936);
        for i in 0..64 {
            assert!((a[0][i] - b1[0][i]).abs() < 1e-5);
        }
        for i in 0..8936 {
            assert!((a[0][64 + i] - b2[0][i]).abs() < 1e-5, "{i}");
        }
    }

    #[test]
    fn the_rear_keeps_its_bass_and_loses_its_presence() {
        let s = with_distance(BinauralDistance::Near);
        let near_ear = |sp: Speaker| {
            let mut m = Monitor::new(&s, ChannelFormat::Atmos714, 48_000);
            let mut main = impulse_on(ChannelFormat::Atmos714, sp, 256);
            m.process(&mut main, 256);
            main[0].clone()
        };
        let (front, rear) = (near_ear(Speaker::L), near_ear(Speaker::Lrs));
        let low = band_db(&rear, 48_000.0, 200.0, 1_000.0) - band_db(&front, 48_000.0, 200.0, 1_000.0);
        let presence = band_db(&rear, 48_000.0, 4_000.0, 10_000.0) - band_db(&front, 48_000.0, 4_000.0, 10_000.0);
        assert!(low.abs() < 1.5, "bass moved by {low} dB");
        assert!(presence < -4.0, "presence moved by {presence} dB");
    }

    #[test]
    fn distance_moves_energy_out_of_the_direct_sound() {
        let rate = 48_000usize;
        let n = rate;
        let wet_ratio = |d: BinauralDistance| {
            let mut m = Monitor::new(&with_distance(d), ChannelFormat::Atmos714, rate as u32);
            let mut main = impulse_on(ChannelFormat::Atmos714, Speaker::C, n);
            m.process(&mut main, n);
            let split = (rate as f32 * 0.005) as usize;
            let direct = energy(&main[0][..split]);
            let wet = energy(&main[0][split..]);
            println!("{d:?}: direct/wet {:.1} dB, total {:.1} dB", 10.0 * (direct / wet.max(1e-12)).log10(), 10.0 * (direct + wet).log10());
            wet / direct
        };
        let (off, near, mid, far) =
            (wet_ratio(BinauralDistance::Off), wet_ratio(BinauralDistance::Near), wet_ratio(BinauralDistance::Mid), wet_ratio(BinauralDistance::Far));
        assert_eq!(off, 0.0);
        assert!(near > 0.0 && near < mid && mid < far, "{near} {mid} {far}");
    }

    #[test]
    fn distance_off_is_a_plain_stereo_feed() {
        let mut m = Monitor::new(&with_distance(BinauralDistance::Off), ChannelFormat::Atmos714, 48_000);
        let mut main = impulse_on(ChannelFormat::Atmos714, Speaker::Lrs, 64);
        m.process(&mut main, 64);
        assert_eq!(main[0][0], 1.0);
        assert!(main[1].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn one_channel_can_have_its_own_distance() {
        let mut s = with_distance(BinauralDistance::Mid);
        s.binaural_distance[2] = BinauralDistance::Off;
        let mut m = Monitor::new(&s, ChannelFormat::Atmos714, 48_000);
        let mut main = impulse_on(ChannelFormat::Atmos714, Speaker::C, 64);
        m.process(&mut main, 64);
        assert!((main[0][0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
    }

    #[test]
    fn stereo_downmix_applies_the_trims() {
        let s = AtmosSettings { monitor: AtmosMonitor::Stereo, surround_trim_db: -6.0, height_trim_db: -12.0, ..AtmosSettings::default() };
        let mut m = Monitor::new(&s, ChannelFormat::Atmos714, 48_000);
        let mut main = impulse_on(ChannelFormat::Atmos714, Speaker::Lss, 4);
        m.process(&mut main, 4);
        assert!((main[0][0] - db(-6.0)).abs() < 1e-4 && main[1][0] == 0.0);
        let mut main = impulse_on(ChannelFormat::Atmos714, Speaker::Rtf, 4);
        m.process(&mut main, 4);
        assert!((main[1][0] - db(-12.0)).abs() < 1e-4 && main[0][0] == 0.0);
        let mut main = impulse_on(ChannelFormat::Atmos714, Speaker::Lfe, 4);
        m.process(&mut main, 4);
        assert!(main.iter().all(|c| c[0] == 0.0), "no LFE in Lo/Ro");
    }

    #[test]
    fn surround_downmix_folds_heights_into_the_layer_below() {
        let s = AtmosSettings { monitor: AtmosMonitor::Surround51, height_trim_db: 0.0, ..AtmosSettings::default() };
        let mut m = Monitor::new(&s, ChannelFormat::Atmos714, 48_000);
        let mut main = impulse_on(ChannelFormat::Atmos714, Speaker::Rtr, 4);
        m.process(&mut main, 4);
        assert_eq!((0..6).map(|c| main[c][0]).collect::<Vec<_>>(), vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    #[ignore = "cost measurement"]
    fn binaural_cost() {
        let mut m = Monitor::new(&with_distance(BinauralDistance::Mid), ChannelFormat::Atmos714, 48_000);
        let block = 512usize;
        let source: Vec<f32> = (0..block).map(|i| (i as f32 * 0.37).sin() * 0.3).collect();
        let started = std::time::Instant::now();
        let rounds = 2_000;
        for _ in 0..rounds {
            let mut main = vec![source.clone(); 12];
            m.process(&mut main, block);
        }
        let seconds = started.elapsed().as_secs_f64();
        let audio = rounds as f64 * block as f64 / 48_000.0;
        println!("{:.2} % of one core for 12 active channels", 100.0 * seconds / audio);
    }
}
