mod drums;
mod notes;

pub use drums::{DrumHit, DrumOptions, classify_hits, drum_hits, hit_candidates};
pub use notes::{Detected, NoteOptions};

use std::sync::OnceLock;
use tract_onnx::prelude::*;

pub const MODEL_RATE: u32 = 22_050;
const WINDOW: usize = 43_844;
const WINDOW_FRAMES: usize = 172;
const OVERLAP_FRAMES: usize = 30;
const HOP_SAMPLES: usize = 256;
const PITCHES: usize = 88;
const CONTOUR_BINS: usize = 264;
const LOWEST_MIDI: u8 = 21;

static MODEL_BYTES: &[u8] = include_bytes!("../models/basic-pitch-nmp.onnx");

type Plan = TypedRunnableModel<TypedModel>;

fn model() -> Result<&'static Plan, String> {
    static MODEL: OnceLock<Result<Plan, String>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            let mut bytes = MODEL_BYTES;
            tract_onnx::onnx()
                .model_for_read(&mut bytes)
                .and_then(|m| m.with_input_fact(0, f32::fact([1, WINDOW, 1]).into()))
                .and_then(|m| m.into_optimized())
                .and_then(|m| m.into_runnable())
                .map_err(|e| format!("audio to MIDI model: {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

#[derive(Debug, Clone, Default)]
pub struct Analysis {
    pub frames: usize,
    pub frame_times: Vec<f64>,
    note: Vec<f32>,
    onset: Vec<f32>,
    contour: Vec<f32>,
}

impl Analysis {
    pub fn note(&self, frame: usize, pitch: usize) -> f32 {
        self.note.get(frame * PITCHES + pitch).copied().unwrap_or(0.0)
    }
    pub fn onset(&self, frame: usize, pitch: usize) -> f32 {
        self.onset.get(frame * PITCHES + pitch).copied().unwrap_or(0.0)
    }
    pub fn contour(&self, frame: usize, bin: usize) -> f32 {
        self.contour.get(frame * CONTOUR_BINS + bin).copied().unwrap_or(0.0)
    }
}

pub fn to_model_rate(mono: &[f32], rate: u32) -> Vec<f32> {
    if rate == MODEL_RATE || rate == 0 {
        return mono.to_vec();
    }
    openpt_dsp::offline::resample(&[mono.to_vec()], rate, MODEL_RATE).into_iter().next().unwrap_or_default()
}

pub fn analyse(mono: &[f32], rate: u32, progress: &mut dyn FnMut(f32) -> bool) -> Result<Analysis, String> {
    let audio = to_model_rate(mono, rate);
    let plan = model()?;
    let overlap = OVERLAP_FRAMES * HOP_SAMPLES;
    let hop = WINDOW - overlap;
    let pad = overlap / 2;
    let mut padded = vec![0.0f32; pad];
    padded.extend_from_slice(&audio);
    let windows = padded.len().div_ceil(hop).max(1);
    let keep = WINDOW_FRAMES - OVERLAP_FRAMES;
    let frame_len = WINDOW as f64 / WINDOW_FRAMES as f64;
    let mut a = Analysis::default();
    for w in 0..windows {
        if !progress(w as f32 / windows as f32) {
            return Err("cancelled".into());
        }
        let start = w * hop;
        let mut x = vec![0.0f32; WINDOW];
        for (d, s) in x.iter_mut().zip(padded.iter().skip(start)) {
            *d = *s;
        }
        let input: Tensor = tract_ndarray::Array3::from_shape_vec((1, WINDOW, 1), x).map_err(|e| e.to_string())?.into();
        let out = plan.run(tvec!(input.into())).map_err(|e| format!("audio to MIDI model: {e}"))?;
        let view = |i: usize| -> Result<Vec<f32>, String> {
            let t = out.get(i).ok_or("audio to MIDI model: missing output")?;
            Ok(t.to_array_view::<f32>().map_err(|e| e.to_string())?.iter().copied().collect())
        };
        let (onset, note, contour) = (view(0)?, view(1)?, view(2)?);
        let half = OVERLAP_FRAMES / 2;
        for j in half..half + keep {
            a.onset.extend(onset.iter().skip(j * PITCHES).take(PITCHES));
            a.note.extend(note.iter().skip(j * PITCHES).take(PITCHES));
            a.contour.extend(contour.iter().skip(j * CONTOUR_BINS).take(CONTOUR_BINS));
            let t = (start as f64 + j as f64 * frame_len - pad as f64) / f64::from(MODEL_RATE);
            a.frame_times.push(t);
        }
    }
    let total = (audio.len() as f64 / frame_len).ceil() as usize;
    a.frames = a.frame_times.len().min(total);
    a.frame_times.truncate(a.frames);
    a.note.truncate(a.frames * PITCHES);
    a.onset.truncate(a.frames * PITCHES);
    a.contour.truncate(a.frames * CONTOUR_BINS);
    progress(1.0);
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, secs: f32, notes: &[f32]) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let env = (t * 40.0).min(1.0) * (-(t % secs) * 0.5).exp();
                notes.iter().map(|m| (t * 440.0 * 2f32.powf((m - 69.0) / 12.0) * std::f32::consts::TAU).sin()).sum::<f32>() * 0.3 * env
                    / notes.len() as f32
            })
            .collect()
    }

    #[test]
    fn a_chord_comes_out_as_its_three_notes() {
        let audio = tone(44_100, 3.0, &[60.0, 64.0, 67.0]);
        let a = analyse(&audio, 44_100, &mut |_| true).unwrap();
        assert!((a.frames as f64 - 3.0 * 86.13).abs() < 4.0, "{} frames", a.frames);
        let notes = a.notes(&NoteOptions::default());
        let mut pitches: Vec<u8> = notes.iter().map(|n| n.pitch).collect();
        pitches.sort_unstable();
        pitches.dedup();
        for p in [60, 64, 67] {
            assert!(pitches.contains(&p), "{p} missing from {pitches:?}");
        }
        let first = notes.iter().map(|n| n.start).fold(f64::MAX, f64::min);
        assert!(first < 0.08, "the chord starts at 0 s, not {first}");
    }

    #[test]
    fn melody_mode_keeps_one_note_at_a_time() {
        let mut audio = tone(48_000, 1.0, &[57.0]);
        audio.extend(tone(48_000, 1.0, &[62.0]));
        audio.extend(tone(48_000, 1.0, &[65.0]));
        let a = analyse(&audio, 48_000, &mut |_| true).unwrap();
        let notes = a.notes(&NoteOptions { mono: true, ..NoteOptions::default() });
        assert_eq!(notes.iter().map(|n| n.pitch).collect::<Vec<_>>(), vec![57, 62, 65], "{notes:?}");
        for w in notes.windows(2) {
            assert!(w[0].end <= w[1].start + 1e-9);
        }
    }
}
