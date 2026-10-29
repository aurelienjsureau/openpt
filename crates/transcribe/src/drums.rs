#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumOptions {
    pub sensitivity: f32,
    pub min_gap_ms: f32,
    pub note: Option<u8>,
}

impl Default for DrumOptions {
    fn default() -> Self {
        DrumOptions { sensitivity: 0.5, min_gap_ms: 45.0, note: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumHit {
    pub at: f64,
    pub note: u8,
    pub velocity: u8,
    pub strength: f32,
}

pub const KICK: u8 = 36;
pub const SNARE: u8 = 38;
pub const HAT: u8 = 42;
pub const OPEN_HAT: u8 = 46;
pub const LOW_TOM: u8 = 45;
pub const MID_TOM: u8 = 47;
pub const HIGH_TOM: u8 = 50;
pub const CRASH: u8 = 49;

struct OnePole {
    a: f32,
    z: f32,
}

impl OnePole {
    fn new(hz: f32, rate: f32) -> Self {
        OnePole { a: (-std::f32::consts::TAU * hz / rate).exp(), z: 0.0 }
    }
    fn low(&mut self, x: f32) -> f32 {
        self.z = x + self.a * (self.z - x);
        self.z
    }
}

struct Bands {
    sub: Vec<f32>,
    low: Vec<f32>,
    mid: Vec<f32>,
    high: Vec<f32>,
}

fn band_envelopes(x: &[f32], rate: f32, hop: usize) -> Bands {
    let lp = |hz: f32| (OnePole::new(hz, rate), OnePole::new(hz, rate));
    let (mut s1, mut s2) = lp(120.0);
    let (mut l1, mut l2) = lp(500.0);
    let (mut m1, mut m2) = lp(4000.0);
    let (mut h1, mut h2) = lp(5000.0);
    let n = x.len().div_ceil(hop.max(1));
    let mut out = Bands { sub: Vec::with_capacity(n), low: Vec::with_capacity(n), mid: Vec::with_capacity(n), high: Vec::with_capacity(n) };
    for chunk in x.chunks(hop.max(1)) {
        let mut acc = [0.0f32; 4];
        for &s in chunk {
            let a = s2.low(s1.low(s));
            let b = l2.low(l1.low(s));
            let c = m2.low(m1.low(s));
            let d = h2.low(h1.low(s));
            for (k, v) in [a, b - a, c - b, s - d].into_iter().enumerate() {
                acc[k] += v * v;
            }
        }
        let len = chunk.len().max(1) as f32;
        out.sub.push((acc[0] / len).sqrt());
        out.low.push((acc[1] / len).sqrt());
        out.mid.push((acc[2] / len).sqrt());
        out.high.push((acc[3] / len).sqrt());
    }
    out
}

fn db(v: f32) -> f32 {
    20.0 * v.max(1e-6).log10()
}

fn velocity_of(peak: f32) -> u8 {
    ((db(peak) + 40.0) / 40.0 * 107.0 + 20.0).clamp(1.0, 127.0) as u8
}

pub fn hit_candidates(mono: &[f32], rate: u32, min_gap_ms: f32) -> Vec<DrumHit> {
    let r = rate.max(1) as f32;
    let hop = ((r / 1000.0).round() as usize).max(1);
    let env: Vec<f32> = mono.chunks(hop).map(|c| c.iter().fold(0.0f32, |m, v| m.max(v.abs()))).collect();
    let top = env.iter().copied().fold(0.0f32, f32::max);
    if top <= 0.0 {
        return Vec::new();
    }
    let bands = band_envelopes(mono, r, hop);
    let at = |e: &[f32], i: usize| e.get(i).copied().unwrap_or(0.0);
    let decay = 10f32.powf(-6.0 / 20.0 / 25.0);
    let gap = min_gap_ms.max(1.0).round() as usize;
    let span = 30usize;
    let mut held = 0.0f32;
    let mut hits: Vec<DrumHit> = Vec::new();
    let mut last: Option<usize> = None;
    for i in 0..env.len() {
        let e = at(&env, i);
        let rising = e > held * 2.0 && e > top * 1e-3;
        held = (held * decay).max(e);
        if !rising {
            continue;
        }
        let peak = (i..(i + span).min(env.len())).map(|j| at(&env, j)).fold(0.0f32, f32::max);
        if let Some(l) = last
            && i - l < gap
        {
            match hits.last() {
                Some(prev) if peak > prev.strength * 1.41 => {
                    hits.pop();
                }
                _ => continue,
            }
        }
        let start = i * hop;
        let first = mono.get(start..(start + hop).min(mono.len())).and_then(|w| w.iter().position(|v| v.abs() >= 0.5 * e)).unwrap_or(0);
        let note = classify_at(&bands, i);
        hits.push(DrumHit { at: (start + first) as f64 / f64::from(r), note, velocity: velocity_of(peak), strength: peak });
        last = Some(i);
    }
    hits
}

fn classify_at(bands: &Bands, i: usize) -> u8 {
    let at = |e: &[f32], i: usize| e.get(i).copied().unwrap_or(0.0);
    let n = bands.high.len();
    let look = i..(i + 40).min(n);
    let energy = |b: &[f32]| look.clone().map(|j| at(b, j) * at(b, j)).sum::<f32>();
    let (su, lo, mi, hi) = (energy(&bands.sub), energy(&bands.low), energy(&bands.mid), energy(&bands.high));
    let total = (su + lo + mi + hi).max(1e-12);
    let early = look.clone().map(|j| at(&bands.high, j)).fold(0.0f32, f32::max);
    let late = ((i + 300)..(i + 600).min(n)).map(|j| at(&bands.high, j)).fold(0.0f32, f32::max);
    classify(su / total, lo / total, mi / total, hi / total, late / early.max(1e-9))
}

pub fn classify_hits(mono: &[f32], rate: u32, times: &[f64]) -> Vec<u8> {
    let r = rate.max(1) as f32;
    let hop = ((r / 1000.0).round() as usize).max(1);
    let bands = band_envelopes(mono, r, hop);
    times.iter().map(|t| classify_at(&bands, ((t.max(0.0) * f64::from(r)) as usize) / hop)).collect()
}

pub fn drum_hits(mono: &[f32], rate: u32, o: &DrumOptions) -> Vec<DrumHit> {
    let mut hits = hit_candidates(mono, rate, o.min_gap_ms);
    let top = hits.iter().map(|h| h.strength).fold(0.0f32, f32::max);
    let floor = top * 10f32.powf(-(6.0 + 30.0 * o.sensitivity.clamp(0.0, 1.0)) / 20.0);
    hits.retain(|h| h.strength >= floor);
    if let Some(n) = o.note {
        hits.iter_mut().for_each(|h| h.note = n);
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit(rate: u32) -> (Vec<f32>, Vec<(f64, u8)>) {
        let r = rate as f32;
        let mut out = vec![0.0f32; rate as usize * 2];
        let mut seed = 12345u32;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
        };
        let plan = [(0.0, KICK), (0.5, SNARE), (0.75, HAT), (1.0, KICK), (1.5, SNARE)];
        for (t, kind) in plan {
            let at = (t * r) as usize;
            let mut prev = 0.0f32;
            for i in 0..(0.25 * r) as usize {
                let x = i as f32 / r;
                let v = match kind {
                    KICK => (x * 55.0 * std::f32::consts::TAU).sin() * (-x * 25.0).exp() * 0.9,
                    SNARE => (noise() * 0.7 + (x * 200.0 * std::f32::consts::TAU).sin() * 0.3) * (-x * 30.0).exp() * 0.8,
                    _ => {
                        let n = noise();
                        let hp = n - prev;
                        prev = n;
                        hp * (-x * 120.0).exp() * 0.4
                    }
                };
                if let Some(o) = out.get_mut(at + i) {
                    *o += v;
                }
            }
        }
        (out, plan.iter().map(|(t, k)| (*t as f64, *k)).collect())
    }

    #[test]
    fn kick_snare_and_hat_land_on_their_own_notes() {
        let (audio, plan) = kit(48_000);
        let hits = drum_hits(&audio, 48_000, &DrumOptions::default());
        let got: Vec<(f64, u8)> = hits.iter().map(|h| (h.at, h.note)).collect();
        assert_eq!(got.len(), plan.len(), "{got:?}");
        for ((t, k), (gt, gk)) in plan.iter().zip(&got) {
            assert_eq!(k, gk, "at {t}: {got:?}");
            assert!((t - gt).abs() < 0.012, "at {t}: detected at {gt}");
        }
    }
}

fn classify(sub: f32, low: f32, mid: f32, high: f32, ring: f32) -> u8 {
    if sub >= 0.3 && sub + low >= 0.8 {
        KICK
    } else if high >= 0.7 && sub + low < 0.1 {
        if ring > 0.6 {
            CRASH
        } else if ring > 0.35 {
            OPEN_HAT
        } else {
            HAT
        }
    } else if mid + high >= 0.45 {
        SNARE
    } else if low + sub > 0.0 {
        let centroid = (sub * 80.0 + low * 250.0 + mid * 1000.0) / (sub + low + mid).max(1e-9);
        if centroid < 160.0 {
            LOW_TOM
        } else if centroid < 260.0 {
            MID_TOM
        } else {
            HIGH_TOM
        }
    } else {
        SNARE
    }
}
