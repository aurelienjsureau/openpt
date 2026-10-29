#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openpt_audio_io::AudioBuffer;
use openpt_mix::MixEngine;
use openpt_model::{ChannelFormat, Clip, Insert, MidiFrom, Route, Session, SourceAudio, SourceId, TrackId, TrackKind};
use std::sync::Arc;

const HITS: [i64; 3] = [9_600, 31_000, 52_300];
const LEN: usize = 72_000;

fn kicks() -> Vec<f32> {
    let mut x = vec![0.0f32; LEN];
    for at in HITS {
        for k in 0..2400 {
            x[at as usize + k] = 0.8 * (-(k as f32) / 300.0).exp() * (std::f32::consts::TAU * 80.0 * k as f32 / 48_000.0).sin();
        }
    }
    x
}

fn fixture(on_aux: bool) -> (Session, TrackId) {
    let mut s = Session::default();
    s.edit.delay_compensation = true;
    let x = kicks();
    s.pool.insert(SourceId(77), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![x.clone(), x] })));
    let kick = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Kick"));
    let clip = s.new_clip_id();
    s.track_mut(kick).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(clip, "kick", SourceId(77), 0, 0, LEN as i64));
    let other = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Other"));
    let mut comp = Insert::new("compressor");
    comp.params.insert("lookahead".into(), 5.0);
    comp.params.insert("ratio".into(), 1.0);
    s.track_mut(other).unwrap().mixer.inserts[0] = Some(comp);
    let drt_on = if on_aux {
        let bus = s.add_bus("Trig", ChannelFormat::Stereo);
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Trig"));
        s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
        s.track_mut(kick).unwrap().mixer.output = Route::Bus(bus);
        aux
    } else {
        kick
    };
    s.track_mut(drt_on).unwrap().mixer.inserts[1] = Some(Insert::new("open_drt"));
    let notes = s.add_track(TrackKind::Midi, ChannelFormat::Stereo, Some("Notes"));
    s.track_mut(notes).unwrap().midi_from = Some(MidiFrom::Plugins(drt_on));
    s.track_mut(notes).unwrap().mixer.record_arm = true;
    (s, notes)
}

fn recorded(s: &Session, notes: TrackId, block: usize) -> Vec<i64> {
    let mut eng = MixEngine::new(48_000.0, block);
    let mut out = vec![vec![0.0f32; block]; 2];
    let mut at = Vec::new();
    let mut pos = 0i64;
    while (pos as usize) < LEN {
        eng.render(s, pos, block, &mut out);
        at.extend(eng.midi_routed_at.iter().filter(|r| r.0 == notes && r.2[0] & 0xF0 == 0x90 && r.2[2] > 0).map(|r| r.1));
        pos += block as i64;
    }
    at
}

#[test]
fn notes_from_a_trigger_plugin_are_recorded_on_the_hits() {
    for on_aux in [false, true] {
        for block in [64, 512, 1024] {
            let (s, notes) = fixture(on_aux);
            let at = recorded(&s, notes, block);
            assert_eq!(at.len(), HITS.len(), "aux {on_aux} block {block}: {at:?}");
            for (got, want) in at.iter().zip(HITS) {
                assert!((got - want).abs() <= 48, "aux {on_aux} block {block}: note at {got}, hit at {want}");
            }
        }
    }
}
