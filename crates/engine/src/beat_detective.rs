use crate::Engine;
use openpt_dsp::biquad::{Biquad, FilterType};
use openpt_model::{ClipId, FadeShape, Session, TrackId};
use openpt_time::{BarBeat, Range, Samples, TICKS_PER_QUARTER};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Bars,
    Beats,
    SubBeats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emphasis {
    Enhanced,
    Low,
    High,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub range: Range,
    pub start: BarBeat,
    pub end: BarBeat,
    pub contains: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trigger {
    pub at: Samples,
    pub tick: i64,
    pub offbeat: bool,
}

fn slots(s: &Session, sel: &Selection, res: Resolution) -> Vec<(i64, Samples, bool)> {
    let t0 = s.tempo.tick_at_bar_beat(sel.start);
    let t1 = s.tempo.tick_at_bar_beat(sel.end);
    if t1 <= t0 || sel.range.is_empty() {
        return Vec::new();
    }
    let span = (t1 - t0) as f64;
    let step = sel.contains.max(1);
    let mut out = Vec::new();
    let mut t = t0;
    let mut k = 0usize;
    while t <= t1 && out.len() < 100_000 {
        let bb = s.tempo.bar_beat_at_tick(t);
        let on_bar = bb.beat == 1 && bb.tick == 0;
        let on_beat = bb.tick == 0;
        let keep = match res {
            Resolution::Bars => on_bar,
            Resolution::Beats => on_beat,
            Resolution::SubBeats => true,
        };
        if keep {
            let at = sel.range.start + ((t - t0) as f64 / span * sel.range.len() as f64).round() as Samples;
            out.push((t, at, k % 2 == 1));
        }
        t += step;
        k += 1;
    }
    out
}

fn attacks(s: &Session, tracks: &[TrackId], r: Range, sens: f32, emphasis: Emphasis) -> Vec<Samples> {
    let sr = s.sample_rate.as_f64() as f32;
    let mut hits = Vec::new();
    for t in tracks {
        if !s.track(*t).is_some_and(|x| x.kind.has_playlist() && !x.kind.is_midi()) {
            continue;
        }
        let r = crate::io::bounded_to_track(s, *t, r);
        let mut audio = openpt_mix::render_clips(s, *t, r);
        let band = match emphasis {
            Emphasis::Low => Some((FilterType::LowPass, 180.0)),
            Emphasis::High => Some((FilterType::HighPass, 2500.0)),
            Emphasis::Enhanced => None,
        };
        if let Some((kind, f)) = band {
            for ch in &mut audio {
                let mut bq = [Biquad::default(), Biquad::default()];
                for b in &mut bq {
                    b.set(kind, f, std::f32::consts::FRAC_1_SQRT_2, 0.0, sr);
                }
                for x in ch.iter_mut() {
                    let y = bq[0].process_sample(*x);
                    *x = bq[1].process_sample(y);
                }
            }
        }
        let pre = (sr * 0.05) as usize;
        for ch in &mut audio {
            ch.splice(0..0, std::iter::repeat_n(0.0, pre));
        }
        hits.extend(openpt_dsp::offline::detect_transients(&audio, sr, sens).into_iter().map(|i| r.start + i as Samples - pre as Samples));
    }
    hits.sort_unstable();
    hits.dedup();
    hits
}

pub fn detect(s: &Session, tracks: &[TrackId], sel: &Selection, res: Resolution, sens: f32, emphasis: Emphasis) -> Vec<Trigger> {
    let wanted = slots(s, sel, res);
    let (Some(first), Some(last)) = (wanted.first(), wanted.last()) else { return Vec::new() };
    let t0 = s.tempo.tick_at_bar_beat(sel.start);
    let t1 = s.tempo.tick_at_bar_beat(sel.end).max(t0 + 1);
    let hits = attacks(s, tracks, sel.range, sens, emphasis);
    let mut rate = sel.range.len() as f64 / (t1 - t0) as f64;
    let step = wanted.windows(2).map(|w| w[1].0 - w[0].0).min().unwrap_or(last.0 - first.0).max(1);
    let mut anchor = (t0, sel.range.start as f64);
    let mut out: Vec<Trigger> = Vec::new();
    for &(tick, _, offbeat) in &wanted {
        let expected = anchor.1 + (tick - anchor.0) as f64 * rate;
        let window = 0.45 * step as f64 * rate;
        let best = hits
            .iter()
            .copied()
            .filter(|h| (*h as f64 - expected).abs() <= window && out.last().is_none_or(|p| *h > p.at))
            .min_by(|a, b| (*a as f64 - expected).abs().total_cmp(&(*b as f64 - expected).abs()));
        let Some(h) = best else { continue };
        if tick > anchor.0 && !out.is_empty() {
            let local = (h as f64 - anchor.1) / (tick - anchor.0) as f64;
            if (local / rate - 1.0).abs() < 0.2 {
                rate = rate * 0.6 + local * 0.4;
            }
        }
        anchor = (tick, h as f64);
        out.push(Trigger { at: h, tick, offbeat });
    }
    out
}

pub fn generate_markers(s: &mut Session, triggers: &[Trigger]) -> Result<usize, String> {
    let sr = s.sample_rate;
    let rate = sr.as_f64();
    let Some(first) = triggers.first() else { return Ok(0) };
    let prev_tick = s.tempo.tempos().iter().map(|t| t.tick).filter(|t| *t < first.tick).max().unwrap_or(0);
    let prev_at = s.tempo.tick_to_samples(prev_tick, sr);
    let bpm = |ticks: i64, samples: Samples| -> Option<f64> {
        (samples > 0 && ticks > 0)
            .then(|| (ticks as f64 / TICKS_PER_QUARTER as f64) * 60.0 * rate / samples as f64)
            .filter(|b| (10.0..=400.0).contains(b))
    };
    let last_tick = triggers.last().map_or(first.tick, |t| t.tick);
    let stale: Vec<i64> = s.tempo.tempos().iter().map(|t| t.tick).filter(|t| *t > prev_tick && *t <= last_tick).collect();
    for t in stale {
        s.tempo.remove_tempo(t);
    }
    let mut made = 0;
    if first.tick > prev_tick
        && let Some(b) = bpm(first.tick - prev_tick, first.at - prev_at)
    {
        s.tempo.set_tempo(prev_tick, b).map_err(|e| e.to_string())?;
        made += 1;
    }
    for w in triggers.windows(2) {
        let (a, b) = (w[0], w[1]);
        if let Some(v) = bpm(b.tick - a.tick, b.at - a.at) {
            s.tempo.set_tempo(a.tick, v).map_err(|e| e.to_string())?;
            made += 1;
        }
    }
    Ok(made)
}

pub fn separate(s: &mut Session, tracks: &[TrackId], triggers: &[Trigger], pad: Samples) -> usize {
    let mut n = 0;
    for t in tracks {
        for tr in triggers {
            n += crate::edit::separate_at(s, *t, tr.at.saturating_sub(pad));
        }
    }
    n
}

#[allow(clippy::too_many_arguments)]
pub fn conform(
    s: &mut Session,
    tracks: &[TrackId],
    triggers: &[Trigger],
    pad: Samples,
    contains: i64,
    strength: f64,
    exclude: f64,
    swing: f64,
) -> usize {
    let sr = s.sample_rate;
    let mut moves: Vec<(ClipId, Samples)> = Vec::new();
    for tr in triggers {
        let slot = (s.tempo.tick_to_samples(tr.tick + contains, sr) - s.tempo.tick_to_samples(tr.tick, sr)).max(1);
        let mut target = s.tempo.tick_to_samples(tr.tick, sr);
        if tr.offbeat {
            target += (swing.clamp(0.0, 1.0) * slot as f64 / 3.0).round() as Samples;
        }
        let off = target - tr.at;
        if (off.abs() as f64) < exclude.clamp(0.0, 1.0) * slot as f64 {
            continue;
        }
        let delta = (off as f64 * strength.clamp(0.0, 1.0)).round() as Samples;
        if delta == 0 {
            continue;
        }
        for t in tracks {
            let Some(track) = s.track(*t) else { continue };
            if let Some(c) = track.clips().iter().find(|c| (c.start - (tr.at - pad)).abs() <= pad.max(1)) {
                moves.push((c.id, delta));
            }
        }
    }
    let n = moves.len();
    for (id, delta) in moves {
        crate::edit::move_clips(s, &[id], delta, &[]);
    }
    n
}

pub fn smooth(s: &mut Session, tracks: &[TrackId], r: Range, xfade: Samples) -> usize {
    let mut n = 0;
    for t in tracks {
        let clips: Vec<(ClipId, Samples, Samples)> =
            s.track(*t).map(|x| x.clips().iter().filter(|c| c.range().overlaps(&r)).map(|c| (c.id, c.start, c.end())).collect()).unwrap_or_default();
        for w in clips.windows(2) {
            let ((a, _, a_end), (_, b_start, _)) = (w[0], w[1]);
            if b_start > a_end {
                crate::edit::trim_clip(s, a, None, Some(b_start), false);
            }
        }
        let bounds: Vec<Samples> =
            s.track(*t).map(|x| x.clips().iter().filter(|c| c.range().overlaps(&r)).map(|c| c.start).collect()).unwrap_or_default();
        if xfade > 0 {
            for b in bounds {
                n += crate::edit::create_fades(s, *t, Range::new(b - xfade / 2, b + xfade / 2), FadeShape::EqualPower);
            }
        }
    }
    n
}

pub fn parse_bar_beat(text: &str) -> Option<BarBeat> {
    let mut it = text.trim().split(['|', ' ', '.']).filter(|x| !x.is_empty());
    let bar = it.next()?.parse::<i64>().ok()?;
    let beat = it.next().map_or(Some(1), |b| b.parse::<i64>().ok())?;
    Some(BarBeat { bar, beat: beat.max(1), tick: 0 })
}

pub fn capture(e: &Engine, r: Range) -> (BarBeat, BarBeat) {
    let s = e.session();
    let round = |at: Samples| {
        let t = s.tempo.samples_to_ticks(at, s.sample_rate);
        let tpb = s.tempo.meter_at_tick(t).ticks_per_beat().max(1);
        let bb = s.tempo.bar_beat_at_tick(((t + tpb / 2) / tpb) * tpb);
        BarBeat { tick: 0, ..bb }
    };
    (round(r.start), round(r.end))
}

pub fn groove_template(s: &Session, triggers: &[Trigger], contains: i64) -> Vec<i64> {
    let sr = s.sample_rate;
    let step = contains.max(1);
    let per_bar = |tick: i64| (s.tempo.meter_at_tick(tick).ticks_per_bar() / step).max(1);
    let n = triggers.first().map_or(1, |t| per_bar(t.tick)) as usize;
    let mut sum = vec![0i64; n];
    let mut count = vec![0i64; n];
    for t in triggers {
        let bar = s.tempo.bar_start_tick(s.tempo.bar_beat_at_tick(t.tick).bar);
        let k = ((t.tick - bar) / step).rem_euclid(n as i64) as usize;
        let played = s.tempo.samples_to_ticks(t.at, sr);
        if let (Some(a), Some(c)) = (sum.get_mut(k), count.get_mut(k)) {
            *a += played - t.tick;
            *c += 1;
        }
    }
    sum.iter().zip(&count).map(|(a, c)| if *c > 0 { a / c } else { 0 }).collect()
}
