use crate::{FrameRate, SampleRate, Samples, TICKS_PER_QUARTER, TempoMap, to_samples};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NoteValue {
    Bar,
    Whole,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    SixtyFourth,
}

impl NoteValue {
    pub const ALL: [NoteValue; 8] = [
        NoteValue::Bar,
        NoteValue::Whole,
        NoteValue::Half,
        NoteValue::Quarter,
        NoteValue::Eighth,
        NoteValue::Sixteenth,
        NoteValue::ThirtySecond,
        NoteValue::SixtyFourth,
    ];
    pub fn ticks(self) -> i64 {
        match self {
            NoteValue::Bar | NoteValue::Whole => TICKS_PER_QUARTER * 4,
            NoteValue::Half => TICKS_PER_QUARTER * 2,
            NoteValue::Quarter => TICKS_PER_QUARTER,
            NoteValue::Eighth => TICKS_PER_QUARTER / 2,
            NoteValue::Sixteenth => TICKS_PER_QUARTER / 4,
            NoteValue::ThirtySecond => TICKS_PER_QUARTER / 8,
            NoteValue::SixtyFourth => TICKS_PER_QUARTER / 16,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            NoteValue::Bar => "1 bar",
            NoteValue::Whole => "1/1 note",
            NoteValue::Half => "1/2 note",
            NoteValue::Quarter => "1/4 note",
            NoteValue::Eighth => "1/8 note",
            NoteValue::Sixteenth => "1/16 note",
            NoteValue::ThirtySecond => "1/32 note",
            NoteValue::SixtyFourth => "1/64 note",
        }
    }
}

pub const FRAMES_PER_FOOT: f64 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GridValue {
    Note { value: NoteValue, dotted: bool, triplet: bool },
    Seconds(f64),
    Frames(f64),
    Feet(f64),
    Samples(i64),
    ClipsMarkers,
}

impl Default for GridValue {
    fn default() -> Self {
        GridValue::Note { value: NoteValue::Sixteenth, dotted: false, triplet: false }
    }
}

impl GridValue {
    pub fn label(&self) -> String {
        match self {
            GridValue::Note { value, dotted, triplet } => {
                let mut s = value.label().to_string();
                if *dotted {
                    s.push_str(" dotted");
                }
                if *triplet {
                    s.push_str(" triplet");
                }
                s
            }
            GridValue::Seconds(s) if *s >= 1.0 => format!("{} sec", trim(*s)),
            GridValue::Seconds(s) => format!("{} msec", trim(s * 1000.0)),
            GridValue::Frames(f) if (*f - 0.5).abs() < 1e-9 => "1/2 frame".into(),
            GridValue::Frames(f) if (*f - 0.25).abs() < 1e-9 => "1/4 frame".into(),
            GridValue::Frames(f) if (*f - 0.01).abs() < 1e-9 => "1 subframe".into(),
            GridValue::Frames(f) => format!("{} frame{}", trim(*f), if (*f - 1.0).abs() < 1e-9 { "" } else { "s" }),
            GridValue::Feet(f) => format!("{} f{}t", trim(*f), if (*f - 1.0).abs() < 1e-9 { "oo" } else { "ee" }),
            GridValue::Samples(n) => format!("{n} sample{}", if *n == 1 { "" } else { "s" }),
            GridValue::ClipsMarkers => "Clips/Markers".into(),
        }
    }

    pub fn scale(&self) -> &'static str {
        match self {
            GridValue::Note { .. } => "bars_beats",
            GridValue::Seconds(_) => "min_secs",
            GridValue::Frames(_) => "timecode",
            GridValue::Feet(_) => "feet_frames",
            GridValue::Samples(_) => "samples",
            GridValue::ClipsMarkers => "clips_markers",
        }
    }

    pub fn stepped(self, larger: bool) -> GridValue {
        fn next<T: PartialOrd + Copy>(ladder: &[T], v: T, larger: bool) -> Option<T> {
            if larger { ladder.iter().copied().find(|x| *x > v) } else { ladder.iter().rev().copied().find(|x| *x < v) }
        }
        match self {
            GridValue::Note { value, dotted, triplet } => {
                let i = NoteValue::ALL.iter().position(|v| *v == value).unwrap_or(0);
                let j = if larger { i.checked_sub(1) } else { i.checked_add(1) };
                j.and_then(|j| NoteValue::ALL.get(j)).map_or(self, |&value| GridValue::Note { value, dotted, triplet })
            }
            GridValue::Seconds(s) => next(&[0.001, 0.01, 0.1, 0.5, 1.0, 10.0], s, larger).map_or(self, GridValue::Seconds),
            GridValue::Frames(f) => next(&[0.01, 0.25, 0.5, 1.0, 5.0, 10.0, 100.0], f, larger).map_or(self, GridValue::Frames),
            GridValue::Feet(f) => next(&[1.0, 10.0], f, larger).map_or(self, GridValue::Feet),
            GridValue::Samples(n) => next(&[1, 2, 10, 100, 1_000, 10_000, 100_000], n, larger).map_or(self, GridValue::Samples),
            GridValue::ClipsMarkers => self,
        }
    }

    fn note_ticks(value: NoteValue, dotted: bool, triplet: bool, map: &TempoMap, at_tick: i64) -> i64 {
        let base = if value == NoteValue::Bar { map.meter_at_tick(at_tick).ticks_per_bar() } else { value.ticks() };
        let mut t = base;
        if dotted {
            t = t * 3 / 2;
        }
        if triplet {
            t = t * 2 / 3;
        }
        t.max(1)
    }

    fn absolute_step(&self, sr: SampleRate, rate: FrameRate) -> Option<f64> {
        match *self {
            GridValue::Seconds(s) => Some(sr.as_f64() * s.clamp(0.000_01, 3600.0)),
            GridValue::Frames(f) => Some(rate.samples_per_frame(sr) * f.clamp(0.01, 1_000_000.0)),
            GridValue::Feet(f) => Some(rate.samples_per_frame(sr) * FRAMES_PER_FOOT * f.clamp(0.01, 100_000.0)),
            GridValue::Samples(n) => Some(n.max(1) as f64),
            GridValue::Note { .. } | GridValue::ClipsMarkers => None,
        }
    }

    pub fn step_samples(&self, at: Samples, sr: SampleRate, map: &TempoMap, rate: FrameRate) -> Samples {
        match *self {
            GridValue::Note { value, dotted, triplet } => {
                let t0 = map.samples_to_ticks(at, sr);
                let dt = Self::note_ticks(value, dotted, triplet, map, t0);
                (map.tick_to_samples(t0 + dt, sr) - map.tick_to_samples(t0, sr)).max(1)
            }
            GridValue::ClipsMarkers => sr.samples(1.0).max(1),
            _ => to_samples(self.absolute_step(sr, rate).unwrap_or(1.0)).max(1),
        }
    }

    pub fn snap(&self, pos: Samples, sr: SampleRate, map: &TempoMap, rate: FrameRate) -> Samples {
        let lo = self.line_at_or_before(pos, sr, map, rate);
        let hi = self.next_line(lo, sr, map, rate);
        if (pos - lo) <= (hi - pos) { lo } else { hi }
    }

    pub fn line_at_or_before(&self, pos: Samples, sr: SampleRate, map: &TempoMap, rate: FrameRate) -> Samples {
        match *self {
            GridValue::Note { value, dotted, triplet } => {
                let mut t = map.samples_to_ticks_f(pos, sr).floor() as i64;
                if map.tick_to_samples(t, sr) > pos {
                    t = t.saturating_sub(1);
                } else if map.tick_to_samples(t.saturating_add(1), sr) <= pos {
                    t = t.saturating_add(1);
                }
                let bb = map.bar_beat_at_tick(t);
                let bar_tick = map.bar_start_tick(bb.bar);
                let step = Self::note_ticks(value, dotted, triplet, map, bar_tick);
                let k = (t - bar_tick).div_euclid(step);
                map.tick_to_samples(bar_tick + k * step, sr)
            }
            GridValue::ClipsMarkers => pos,
            _ => {
                let step = self.absolute_step(sr, rate).unwrap_or(1.0).max(1.0);
                let k = (pos as f64 / step).floor();
                let mut line = to_samples(k * step);
                if line > pos {
                    line = to_samples((k - 1.0) * step);
                }
                line
            }
        }
    }

    pub fn next_line(&self, line: Samples, sr: SampleRate, map: &TempoMap, rate: FrameRate) -> Samples {
        if let Some(st) = self.absolute_step(sr, rate) {
            let st = st.max(1.0);
            let k = (line as f64 / st).floor();
            let mut n = to_samples((k + 1.0) * st);
            if n <= line {
                n = to_samples((k + 2.0) * st);
            }
            return n.max(line.saturating_add(1));
        }
        let step = self.step_samples(line, sr, map, rate);
        let candidate = self.line_at_or_before(line.saturating_add(step), sr, map, rate);
        if candidate > line { candidate } else { line.saturating_add(step) }
    }

    pub fn lines(&self, start: Samples, end: Samples, sr: SampleRate, map: &TempoMap, rate: FrameRate, max: usize) -> Vec<Samples> {
        let mut out = Vec::new();
        if *self == GridValue::ClipsMarkers {
            return out;
        }
        let mut p = self.line_at_or_before(start, sr, map, rate);
        while p < end && out.len() < max {
            if p >= start {
                out.push(p);
            }
            let n = self.next_line(p, sr, map, rate);
            if n <= p {
                break;
            }
            p = n;
        }
        out
    }

    pub fn snap_with(&self, pos: Samples, sr: SampleRate, map: &TempoMap, rate: FrameRate, anchors: &[Samples]) -> Samples {
        if *self != GridValue::ClipsMarkers {
            return self.snap(pos, sr, map, rate);
        }
        anchors.iter().copied().min_by_key(|a| (a - pos).unsigned_abs()).unwrap_or(pos)
    }

    pub fn line_at_or_before_with(&self, pos: Samples, sr: SampleRate, map: &TempoMap, rate: FrameRate, anchors: &[Samples]) -> Samples {
        if *self != GridValue::ClipsMarkers {
            return self.line_at_or_before(pos, sr, map, rate);
        }
        anchors.iter().copied().take_while(|a| *a <= pos).last().unwrap_or(pos)
    }

    pub fn lines_with(
        &self,
        start: Samples,
        end: Samples,
        sr: SampleRate,
        map: &TempoMap,
        rate: FrameRate,
        max: usize,
        anchors: &[Samples],
    ) -> Vec<Samples> {
        if *self != GridValue::ClipsMarkers {
            return self.lines(start, end, sr, map, rate, max);
        }
        anchors.iter().copied().filter(|a| *a >= start && *a < end).take(max).collect()
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_lines_are_their_own_line_at_odd_tempos() {
        let (g, sr, rate) = (GridValue::default(), SampleRate::HZ_48000, FrameRate::Fps30);
        for bpm in [90.0, 95.0, 110.0, 130.0, 140.0] {
            let map = TempoMap::new(bpm, 4, 4).unwrap();
            let mut line = 0;
            for _ in 0..64 {
                assert_eq!(g.line_at_or_before(line, sr, &map, rate), line, "{bpm} bpm");
                line = g.next_line(line, sr, &map, rate);
            }
        }
    }

    #[test]
    fn sixteenth_grid_at_120() {
        let g = GridValue::default();
        let (sr, map, rate) = (SampleRate::HZ_48000, TempoMap::default(), FrameRate::Fps30);
        assert_eq!(g.step_samples(0, sr, &map, rate), 6000);
        assert_eq!(g.snap(2900, sr, &map, rate), 0);
        assert_eq!(g.snap(3100, sr, &map, rate), 6000);
        assert_eq!(g.lines(0, 24_000, sr, &map, rate, 100), vec![0, 6000, 12000, 18000]);
    }

    #[test]
    fn absolute_grids() {
        let (sr, map, rate) = (SampleRate::HZ_48000, TempoMap::default(), FrameRate::Fps25);
        assert_eq!(GridValue::Seconds(1.0).snap(70_000, sr, &map, rate), 48_000);
        assert_eq!(GridValue::Frames(1.0).step_samples(0, sr, &map, rate), 1920);
        assert_eq!(GridValue::Samples(0).step_samples(0, sr, &map, rate), 1);
    }

    #[test]
    fn lines_are_capped() {
        let (sr, map, rate) = (SampleRate::HZ_48000, TempoMap::default(), FrameRate::Fps25);
        assert_eq!(GridValue::Samples(1).lines(0, 1_000_000, sr, &map, rate, 50).len(), 50);
    }

    #[test]
    fn triplets_and_dots() {
        let (sr, map, rate) = (SampleRate::HZ_48000, TempoMap::default(), FrameRate::Fps25);
        let t = GridValue::Note { value: NoteValue::Quarter, dotted: false, triplet: true };
        assert_eq!(t.step_samples(0, sr, &map, rate), 16_000);
        let d = GridValue::Note { value: NoteValue::Quarter, dotted: true, triplet: false };
        assert_eq!(d.step_samples(0, sr, &map, rate), 36_000);
    }

    #[test]
    fn frame_fractions_feet_whole_notes_and_clips_markers() {
        let (sr, map, rate) = (SampleRate::HZ_48000, TempoMap::default(), FrameRate::Fps25);
        assert_eq!(GridValue::Frames(0.5).step_samples(0, sr, &map, rate), 960);
        assert_eq!(GridValue::Frames(0.01).step_samples(0, sr, &map, rate), 19);
        assert_eq!(GridValue::Frames(0.01).label(), "1 subframe");
        assert_eq!(GridValue::Feet(1.0).step_samples(0, sr, &map, rate), 1920 * 16);
        assert_eq!(GridValue::Feet(1.0).label(), "1 foot");
        assert_eq!(GridValue::Seconds(0.5).label(), "500 msec");
        assert_eq!(GridValue::Samples(2).label(), "2 samples");
        let lines = GridValue::Frames(0.01).lines(0, 192_000, sr, &map, rate, 100_000);
        assert_eq!(lines.len(), 10_000);
        assert_eq!(lines.last().copied(), Some(to_samples(9_999.0 * 19.2)));
        let odd = TempoMap::new(120.0, 3, 4).unwrap();
        let whole = GridValue::Note { value: NoteValue::Whole, dotted: false, triplet: false };
        let bar = GridValue::Note { value: NoteValue::Bar, dotted: false, triplet: false };
        assert_eq!(whole.step_samples(0, sr, &odd, rate), 96_000);
        assert_eq!(bar.step_samples(0, sr, &odd, rate), 72_000);
        let anchors = [1000, 5000, 9000];
        let cm = GridValue::ClipsMarkers;
        assert_eq!(cm.snap_with(4200, sr, &map, rate, &anchors), 5000);
        assert_eq!(cm.line_at_or_before_with(8999, sr, &map, rate, &anchors), 5000);
        assert_eq!(cm.lines_with(2000, 10_000, sr, &map, rate, 10, &anchors), vec![5000, 9000]);
        assert!(cm.lines(0, 10_000, sr, &map, rate, 10).is_empty());
    }
    #[test]
    fn values_step_to_the_next_larger_or_smaller() {
        let note = |value, triplet| GridValue::Note { value, dotted: false, triplet };
        assert_eq!(note(NoteValue::Sixteenth, false).stepped(true), note(NoteValue::Eighth, false));
        assert_eq!(note(NoteValue::Eighth, true).stepped(false), note(NoteValue::Sixteenth, true));
        assert_eq!(note(NoteValue::Bar, false).stepped(true), note(NoteValue::Bar, false));
        assert_eq!(note(NoteValue::SixtyFourth, false).stepped(false), note(NoteValue::SixtyFourth, false));
        assert_eq!(GridValue::Seconds(0.01).stepped(true), GridValue::Seconds(0.1));
        assert_eq!(GridValue::Seconds(0.5).stepped(false), GridValue::Seconds(0.1));
        assert_eq!(GridValue::Seconds(10.0).stepped(true), GridValue::Seconds(10.0));
        assert_eq!(GridValue::Frames(1.0).stepped(true), GridValue::Frames(5.0));
        assert_eq!(GridValue::Samples(100).stepped(false), GridValue::Samples(10));
        assert_eq!(GridValue::Samples(1).stepped(false), GridValue::Samples(1));
    }
}
