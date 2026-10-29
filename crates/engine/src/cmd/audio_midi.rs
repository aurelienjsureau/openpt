use super::*;
use crate::{Clipboard, cmd};
use openpt_midi::{CtrlEvent, CtrlKind, Note, Sequence};
use openpt_model::{Clip, ClipId, Session, TrackId};
use openpt_time::Range;
use serde_json::json;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(noundo "edit.copy_audio_as_midi", "Audio as MIDI", ["Edit", "Copy Special"], None, "{track?, start?, end?, mode?: mono|poly|drums} — pitch tracking of the selected audio onto the clipboard", has_range, audio_as_midi),
        cmd!(
            "event.audio_to_midi",
            "Audio to MIDI",
            [],
            None,
            "{track?, start?, end?, mode?: poly|mono|drums, onset?: 0.5, frame?: 0.3, min_ms?: 127.7, low?: 21, high?: 108, bends?: false, sensitivity?: 0.5, drum_note?: 36, notes?: [[start_s, end_s, pitch, velocity], …]} — a new MIDI track under the audio",
            has_range,
            audio_to_midi
        ),
    ]
}

#[derive(Debug, Clone, PartialEq)]
pub struct MidiNote {
    pub start: f64,
    pub end: f64,
    pub pitch: u8,
    pub velocity: u8,
    pub bends: Vec<(f64, f32)>,
}

pub fn selected_mono(s: &Session, t: TrackId, r: Range) -> Vec<f32> {
    let audio = openpt_mix::render_clips(s, t, r);
    let n = audio.first().map_or(0, Vec::len);
    let k = audio.len().max(1) as f32;
    (0..n).map(|i| audio.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>() / k).collect()
}

fn source_range(e: &Engine, id: &str, p: &Value) -> Result<(TrackId, Range)> {
    let t = tracks_required(e, id, p)?.first().copied().ok_or_else(|| bad(id, "no track"))?;
    let r = range_param(e, id, p)?;
    if r.is_empty() {
        return Err(bad(id, "select some audio first"));
    }
    let s = e.session();
    if s.track(t).is_none_or(|x| x.kind != openpt_model::TrackKind::Audio) {
        return Err(bad(id, "select audio on an audio track"));
    }
    Ok((t, crate::io::bounded_to_track(s, t, r)))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn drt_hits(mono: &[f32], rate: u32, sensitivity: f32, retrigger_ms: f32) -> Vec<openpt_transcribe::DrumHit> {
    use openpt_dsp::Plugin;
    const BLOCK: usize = 1024;
    let mut d = openpt_dsp::plugins::open_drt::OpenDrt::new();
    d.prepare(rate as f32, BLOCK, 1);
    for (id, v) in [("sensitivity", sensitivity), ("retrigger", retrigger_ms), ("floor", -72.0), ("outmode", 2.0), ("dynvel", 1.0), ("focus", 0.0)] {
        d.set_param(id, v);
    }
    let la = d.latency();
    let mut times = Vec::new();
    let mut io = vec![vec![0.0f32; BLOCK]];
    let mut out = Vec::with_capacity(256);
    let total = mono.len() + la + BLOCK;
    let mut pos = 0;
    while pos < total {
        let n = BLOCK.min(total - pos);
        for (k, v) in io[0].iter_mut().enumerate().take(n) {
            *v = mono.get(pos + k).copied().unwrap_or(0.0);
        }
        d.process(&mut io, n);
        out.clear();
        d.take_midi_out(&mut out);
        times.extend(out.iter().filter(|(_, m)| m[0] & 0xF0 == 0x90 && m[2] > 0).map(|(o, _)| (pos + o).saturating_sub(la)));
        pos += n;
    }
    let r = f64::from(rate.max(1));
    let secs: Vec<f64> = times.iter().map(|&t| t as f64 / r).collect();
    let notes = openpt_transcribe::classify_hits(mono, rate, &secs);
    let span = (0.03 * r) as usize;
    times
        .iter()
        .zip(secs)
        .zip(notes)
        .map(|((&t, at), note)| {
            let peak = mono.get(t..(t + span).min(mono.len())).unwrap_or(&[]).iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let db = 20.0 * peak.max(1e-6).log10();
            let velocity = ((db + 40.0) / 40.0 * 107.0 + 20.0).clamp(1.0, 127.0) as u8;
            openpt_transcribe::DrumHit { at, note, velocity, strength: peak }
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn detect(mono: &[f32], rate: u32, p: &Value) -> std::result::Result<Vec<MidiNote>, String> {
    let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d) as f32;
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("poly");
    if mode == "drums" {
        let note = p.get("drum_note").and_then(Value::as_u64).filter(|n| (1..=127).contains(n)).map(|n| n as u8);
        let floor = 10f32.powf(f("threshold_db", -45.0) / 20.0);
        return Ok(drt_hits(mono, rate, f("sensitivity", 0.55) * 100.0, f("retrigger_ms", 40.0))
            .into_iter()
            .filter(|h| h.strength >= floor)
            .map(|h| MidiNote { start: h.at, end: h.at + 0.1, pitch: note.unwrap_or(h.note), velocity: h.velocity, bends: Vec::new() })
            .collect());
    }
    let a = openpt_transcribe::analyse(mono, rate, &mut |_| true)?;
    Ok(notes_from(&a, &options(p)))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn options(p: &Value) -> openpt_transcribe::NoteOptions {
    let f = |k: &str, d: f32| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(d, |v| v as f32);
    let d = openpt_transcribe::NoteOptions::default();
    let pitch = |k: &str, d: u8| p.get(k).and_then(Value::as_u64).map_or(d, |v| v.clamp(21, 108) as u8);
    openpt_transcribe::NoteOptions {
        onset_threshold: f("onset", d.onset_threshold).clamp(0.05, 0.95),
        frame_threshold: f("frame", d.frame_threshold).clamp(0.05, 0.95),
        min_note_ms: f("min_ms", d.min_note_ms).clamp(10.0, 2000.0),
        min_pitch: pitch("low", d.min_pitch),
        max_pitch: pitch("high", d.max_pitch),
        mono: p.get("mode").and_then(Value::as_str) == Some("mono"),
        pitch_bends: p.get("bends").and_then(Value::as_bool).unwrap_or(false),
        ..d
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn notes_from(a: &openpt_transcribe::Analysis, o: &openpt_transcribe::NoteOptions) -> Vec<MidiNote> {
    a.notes(o).into_iter().map(|n| MidiNote { start: n.start, end: n.end, pitch: n.pitch, velocity: n.velocity, bends: n.bends }).collect()
}

#[cfg(target_arch = "wasm32")]
pub fn detect(mono: &[f32], rate: u32, _: &Value) -> std::result::Result<Vec<MidiNote>, String> {
    Ok(openpt_dsp::pitch_detect::audio_to_notes(mono, rate as f32)
        .into_iter()
        .map(|d| MidiNote { start: d.start as f64 / f64::from(rate), end: d.end as f64 / f64::from(rate), pitch: d.pitch, velocity: d.velocity, bends: Vec::new() })
        .collect())
}

fn given_notes(p: &Value) -> Option<Vec<MidiNote>> {
    let list = p.get("notes")?.as_array()?;
    Some(
        list.iter()
            .filter_map(|n| {
                let a = n.as_array()?;
                let num = |i: usize| a.get(i).and_then(Value::as_f64);
                Some(MidiNote {
                    start: num(0)?.max(0.0),
                    end: num(1)?,
                    pitch: num(2)?.clamp(0.0, 127.0) as u8,
                    velocity: num(3).unwrap_or(100.0).clamp(1.0, 127.0) as u8,
                    bends: a
                        .get(4)
                        .and_then(Value::as_array)
                        .map(|b| {
                            b.iter()
                                .filter_map(|x| {
                                    let x = x.as_array()?;
                                    Some((x.first()?.as_f64()?, x.get(1)?.as_f64()? as f32))
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
            })
            .filter(|n| n.end > n.start)
            .collect(),
    )
}

fn sequence(s: &Session, r: Range, notes: &[MidiNote]) -> Sequence {
    let rate = s.sample_rate.as_f64();
    let base = s.tempo.samples_to_ticks(r.start, s.sample_rate);
    let tick = |secs: f64| s.tempo.samples_to_ticks(r.start + (secs * rate).round() as i64, s.sample_rate) - base;
    let mut seq = Sequence::default();
    for n in notes {
        let (a, b) = (tick(n.start), tick(n.end));
        seq.notes.push(Note { pitch: n.pitch, velocity: n.velocity, release_velocity: 64, channel: 0, start: a.max(0), length: (b - a).max(1) });
    }
    let overlaps = |i: usize| {
        let n = &notes[i];
        notes.iter().enumerate().any(|(j, m)| j != i && m.start < n.end && n.start < m.end)
    };
    for (i, n) in notes.iter().enumerate() {
        if n.bends.is_empty() || overlaps(i) {
            continue;
        }
        for (t, semis) in &n.bends {
            let value = (f64::from(*semis) / 2.0 * 8192.0).round().clamp(-8192.0, 8191.0) as i32;
            seq.ctrls.push(CtrlEvent { tick: tick(*t).max(0), channel: 0, kind: CtrlKind::PitchBend, value });
        }
        seq.ctrls.push(CtrlEvent { tick: tick(n.end).max(0), channel: 0, kind: CtrlKind::PitchBend, value: 0 });
    }
    seq.sort();
    seq
}

fn audio_as_midi(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "edit.copy_audio_as_midi";
    let (t, r) = source_range(e, id, p)?;
    let s = e.session();
    let mono = selected_mono(s, t, r);
    let mut q = p.clone();
    if q.get("mode").is_none()
        && let Some(o) = q.as_object_mut()
    {
        o.insert("mode".into(), json!("mono"));
    }
    let notes = detect(&mono, s.sample_rate.hz(), &q).map_err(|err| bad(id, err))?;
    let seq = sequence(s, r, &notes);
    let count = seq.notes.len();
    let clip = Clip::midi(ClipId(0), "Audio as MIDI", 0, r.len(), seq);
    e.clipboard = Clipboard { tracks: vec![vec![clip]], automation: vec![Vec::new()], ..Clipboard::of_range(e.session(), r) };
    Ok(json!({"notes": count, "length": r.len()}))
}

fn audio_to_midi(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "event.audio_to_midi";
    let (t, r) = source_range(e, id, p)?;
    let notes = match given_notes(p) {
        Some(n) => n,
        None => {
            let s = e.session();
            let mono = selected_mono(s, t, r);
            detect(&mono, s.sample_rate.hz(), p).map_err(|err| bad(id, err))?
        }
    };
    let s = e.session_mut();
    let seq = sequence(s, r, &notes);
    let count = seq.notes.len();
    let name = s.track(t).map_or_else(|| "Audio".to_string(), |x| x.name.clone());
    let after = s.track_index(t);
    let m = s.add_track(openpt_model::TrackKind::Midi, openpt_model::ChannelFormat::Mono, Some(&format!("{name} MIDI")));
    super::track::place_after(s, &[m], after);
    let cid = s.new_clip_id();
    let clip_name = match p.get("mode").and_then(Value::as_str) {
        Some("drums") => format!("{name} Drums"),
        _ => format!("{name} MIDI"),
    };
    crate::edit::place_clip(s, m, Clip::midi(cid, clip_name, r.start, r.len(), seq));
    s.edit.selected_tracks = vec![m];
    Ok(json!({"track": m, "clip": cid, "notes": count}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use openpt_audio_io::AudioBuffer;
    use openpt_model::{ChannelFormat, ClipContent, TrackKind};

    fn melody(e: &mut Engine) -> (TrackId, i64) {
        let sr = 48_000.0f32;
        let tone = |f: f32| -> Vec<f32> { (0..(0.4 * sr) as usize).map(|i| (i as f32 * f * std::f32::consts::TAU / sr).sin() * 0.5).collect() };
        let mut a = tone(220.0);
        a.extend(tone(330.0));
        let frames = a.len() as i64;
        let s = e.session_mut();
        let src = crate::io::add_source(s, "mel", AudioBuffer { sample_rate: 48_000, channels: vec![a] }, None, openpt_audio_io::FileFormat::Wav);
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Lead"));
        let cid = s.new_clip_id();
        crate::edit::place_clip(s, t, Clip::audio(cid, "mel", src, 0, 48_000, frames));
        (t, frames)
    }

    fn pitches(seq: &ClipContent) -> Vec<u8> {
        match seq {
            ClipContent::Midi { sequence } => sequence.notes.iter().map(|n| n.pitch).collect(),
            _ => panic!("expected MIDI"),
        }
    }

    #[test]
    fn copies_a_melody_as_midi_and_pastes_it() {
        let mut e = Engine::default();
        let (t, frames) = melody(&mut e);
        let r = e.execute("edit.copy_audio_as_midi", &json!({"track": t.0, "start": 48_000, "end": 48_000 + frames})).unwrap();
        assert_eq!(r["notes"], 2, "{r}");
        let m = e.session_mut().add_track(TrackKind::Midi, ChannelFormat::Mono, None);
        e.execute("edit.paste", &json!({"track": m.0, "at": 0})).unwrap();
        let clip = e.session().track(m).unwrap().clips()[0].clone();
        assert_eq!(pitches(&clip.content), vec![57, 64]);
    }

    #[test]
    fn audio_to_midi_makes_a_track_under_the_audio() {
        let mut e = Engine::default();
        let (t, frames) = melody(&mut e);
        e.execute("track.new", &json!({"name": "Other"})).unwrap();
        let r = e.execute("event.audio_to_midi", &json!({"track": t.0, "start": 48_000, "end": 48_000 + frames, "mode": "mono"})).unwrap();
        let m = TrackId(r["track"].as_u64().unwrap());
        let s = e.session();
        assert_eq!(s.track_index(m), s.track_index(t).map(|i| i + 1));
        assert_eq!(s.track(m).unwrap().name, "Lead MIDI");
        let clip = &s.track(m).unwrap().clips()[0];
        assert_eq!((clip.start, clip.length), (48_000, frames));
        assert_eq!(pitches(&clip.content), vec![57, 64]);
    }

    #[test]
    fn edited_notes_from_the_preview_are_written_as_given() {
        let mut e = Engine::default();
        let (t, frames) = melody(&mut e);
        let r = e
            .execute("event.audio_to_midi", &json!({"track": t.0, "start": 48_000, "end": 48_000 + frames, "notes": [[0.0, 0.4, 60, 90], [0.4, 0.8, 67, 80]]}))
            .unwrap();
        let m = TrackId(r["track"].as_u64().unwrap());
        assert_eq!(pitches(&e.session().track(m).unwrap().clips()[0].content), vec![60, 67]);
    }
}
