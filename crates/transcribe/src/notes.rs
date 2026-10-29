use crate::{Analysis, CONTOUR_BINS, LOWEST_MIDI, PITCHES};

const ENERGY_TOLERANCE: usize = 11;
const BINS_PER_SEMITONE: f64 = 3.0;
const BEND_TOLERANCE: usize = 25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteOptions {
    pub onset_threshold: f32,
    pub frame_threshold: f32,
    pub min_note_ms: f32,
    pub min_pitch: u8,
    pub max_pitch: u8,
    pub infer_onsets: bool,
    pub melodia: bool,
    pub mono: bool,
    pub pitch_bends: bool,
}

impl Default for NoteOptions {
    fn default() -> Self {
        NoteOptions {
            onset_threshold: 0.5,
            frame_threshold: 0.3,
            min_note_ms: 127.7,
            min_pitch: LOWEST_MIDI,
            max_pitch: LOWEST_MIDI + PITCHES as u8 - 1,
            infer_onsets: true,
            melodia: true,
            mono: false,
            pitch_bends: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Detected {
    pub start: f64,
    pub end: f64,
    pub pitch: u8,
    pub velocity: u8,
    pub amplitude: f32,
    pub bends: Vec<(f64, f32)>,
}

struct Grid {
    rows: usize,
    data: Vec<f32>,
}

impl Grid {
    fn at(&self, t: usize, p: usize) -> f32 {
        self.data.get(t * PITCHES + p).copied().unwrap_or(0.0)
    }
    fn set(&mut self, t: usize, p: usize, v: f32) {
        if let Some(x) = self.data.get_mut(t * PITCHES + p) {
            *x = v;
        }
    }
}

impl Analysis {
    fn grid(&self, f: impl Fn(&Analysis, usize, usize) -> f32, lo: usize, hi: usize) -> Grid {
        let mut data = vec![0.0f32; self.frames * PITCHES];
        for t in 0..self.frames {
            for p in lo..=hi.min(PITCHES - 1) {
                if let Some(x) = data.get_mut(t * PITCHES + p) {
                    *x = f(self, t, p);
                }
            }
        }
        Grid { rows: self.frames, data }
    }

    pub fn notes(&self, o: &NoteOptions) -> Vec<Detected> {
        let n = self.frames;
        if n < 3 {
            return Vec::new();
        }
        let lo = usize::from(o.min_pitch.max(LOWEST_MIDI) - LOWEST_MIDI);
        let hi = usize::from(o.max_pitch.max(o.min_pitch).saturating_sub(LOWEST_MIDI)).min(PITCHES - 1);
        let frames = self.grid(Analysis::note, lo, hi);
        let mut onsets = self.grid(Analysis::onset, lo, hi);
        if o.infer_onsets {
            infer_onsets(&mut onsets, &frames);
        }
        let fps = n as f64 / (self.frame_times.last().copied().unwrap_or(1.0) - self.frame_times.first().copied().unwrap_or(0.0)).max(1e-3);
        let min_len = ((f64::from(o.min_note_ms) / 1000.0) * fps).round() as usize;
        let mut remaining = Grid { rows: n, data: frames.data.clone() };
        let mut events: Vec<(usize, usize, usize, f32)> = Vec::new();
        let mut peaks: Vec<(usize, usize)> = Vec::new();
        for t in 1..n - 1 {
            for p in lo..=hi {
                let v = onsets.at(t, p);
                if v >= o.onset_threshold && v > onsets.at(t - 1, p) && v > onsets.at(t + 1, p) {
                    peaks.push((t, p));
                }
            }
        }
        peaks.sort_by(|a, b| b.cmp(a));
        let clear = |rem: &mut Grid, t: usize, p: usize| {
            rem.set(t, p, 0.0);
            if p + 1 < PITCHES {
                rem.set(t, p + 1, 0.0);
            }
            if p > 0 {
                rem.set(t, p - 1, 0.0);
            }
        };
        for (start, p) in peaks {
            if start >= n - 1 {
                continue;
            }
            let (mut i, mut k) = (start + 1, 0);
            while i < n - 1 && k < ENERGY_TOLERANCE {
                k = if remaining.at(i, p) < o.frame_threshold { k + 1 } else { 0 };
                i += 1;
            }
            i -= k;
            if i - start <= min_len {
                continue;
            }
            for t in start..i {
                clear(&mut remaining, t, p);
            }
            events.push((start, i, p, mean(&frames, start, i, p)));
        }
        if o.melodia {
            let mut candidates: Vec<(usize, usize, f32)> = Vec::new();
            for t in 0..n {
                for p in lo..=hi {
                    let v = remaining.at(t, p);
                    if v > o.frame_threshold {
                        candidates.push((t, p, v));
                    }
                }
            }
            candidates.sort_by(|a, b| b.2.total_cmp(&a.2));
            for (mid, p, v) in candidates {
                if remaining.at(mid, p) != v {
                    continue;
                }
                remaining.set(mid, p, 0.0);
                let (mut i, mut k) = (mid + 1, 0);
                while i < n - 1 && k < ENERGY_TOLERANCE {
                    k = if remaining.at(i, p) < o.frame_threshold { k + 1 } else { 0 };
                    clear(&mut remaining, i, p);
                    i += 1;
                }
                let end = i.saturating_sub(1 + k);
                let (mut i, mut k) = (mid as isize - 1, 0);
                while i > 0 && k < ENERGY_TOLERANCE {
                    let iu = i as usize;
                    k = if remaining.at(iu, p) < o.frame_threshold { k + 1 } else { 0 };
                    clear(&mut remaining, iu, p);
                    i -= 1;
                }
                let start = (i + 1 + k as isize).max(0) as usize;
                if end <= start || end - start <= min_len {
                    continue;
                }
                events.push((start, end, p, mean(&frames, start, end, p)));
            }
        }
        if o.mono {
            events = monophonic(events, min_len);
        }
        let time = |f: usize| self.frame_times.get(f.min(n - 1)).copied().unwrap_or(0.0).max(0.0);
        let mut out: Vec<Detected> = events
            .into_iter()
            .map(|(s, e, p, amp)| Detected {
                start: time(s),
                end: time(e),
                pitch: LOWEST_MIDI + p as u8,
                velocity: (127.0 * amp).round().clamp(1.0, 127.0) as u8,
                amplitude: amp,
                bends: if o.pitch_bends { self.bends(s, e, p, &time) } else { Vec::new() },
            })
            .collect();
        out.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        out
    }

    fn bends(&self, s: usize, e: usize, p: usize, time: &dyn Fn(usize) -> f64) -> Vec<(f64, f32)> {
        let centre = (p as f64 * BINS_PER_SEMITONE).round() as isize;
        let tol = BEND_TOLERANCE as isize;
        let gauss = |d: isize| (-(d as f64 * d as f64) / (2.0 * 25.0)).exp() as f32;
        let mut out = Vec::new();
        let mut last = f32::NAN;
        for t in s..e {
            let mut best = (0isize, f32::MIN);
            for d in -tol..=tol {
                let b = centre + d;
                if b < 0 || b >= CONTOUR_BINS as isize {
                    continue;
                }
                let v = self.contour(t, b as usize) * gauss(d);
                if v > best.1 {
                    best = (d, v);
                }
            }
            let semis = (best.0 as f64 / BINS_PER_SEMITONE) as f32;
            if (semis - last).abs() > 1e-6 || last.is_nan() {
                out.push((time(t), semis));
                last = semis;
            }
        }
        if out.iter().all(|(_, b)| b.abs() < 1e-6) { Vec::new() } else { out }
    }
}

fn mean(g: &Grid, s: usize, e: usize, p: usize) -> f32 {
    let n = e.saturating_sub(s).max(1);
    (s..e).map(|t| g.at(t, p)).sum::<f32>() / n as f32
}

fn infer_onsets(onsets: &mut Grid, frames: &Grid) {
    let n = frames.rows;
    let mut diff = vec![0.0f32; n * PITCHES];
    let mut max_diff = 0.0f32;
    for t in 2..n {
        for p in 0..PITCHES {
            let d1 = frames.at(t, p) - frames.at(t - 1, p);
            let d2 = frames.at(t, p) - frames.at(t - 2, p);
            let d = d1.min(d2).max(0.0);
            if let Some(x) = diff.get_mut(t * PITCHES + p) {
                *x = d;
            }
            max_diff = max_diff.max(d);
        }
    }
    let max_onset = onsets.data.iter().copied().fold(0.0f32, f32::max);
    if max_diff <= 0.0 {
        return;
    }
    for (o, d) in onsets.data.iter_mut().zip(diff) {
        *o = o.max(max_onset * d / max_diff);
    }
}

fn monophonic(mut events: Vec<(usize, usize, usize, f32)>, min_len: usize) -> Vec<(usize, usize, usize, f32)> {
    events.sort_by(|a, b| b.3.total_cmp(&a.3));
    let mut kept: Vec<(usize, usize, usize, f32)> = Vec::new();
    for (s, e, p, a) in events {
        let mut pieces = vec![(s, e)];
        for &(ks, ke, _, _) in &kept {
            pieces = pieces
                .into_iter()
                .flat_map(|(a0, a1)| {
                    if ke <= a0 || ks >= a1 {
                        vec![(a0, a1)]
                    } else {
                        let mut v = Vec::new();
                        if ks > a0 {
                            v.push((a0, ks));
                        }
                        if ke < a1 {
                            v.push((ke, a1));
                        }
                        v
                    }
                })
                .collect();
        }
        for (a0, a1) in pieces {
            if a1 - a0 > min_len {
                kept.push((a0, a1, p, a));
            }
        }
    }
    kept.sort_by_key(|x| x.0);
    kept
}
