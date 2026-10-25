use openpt_audio_io::AudioBuffer;
use openpt_model::{ChannelFormat, Clip, Insert, Session, SourceAudio, SourceId, TrackKind};
use openpt_time::Range;
use std::sync::Arc;

#[test]
fn the_silence_after_a_note_costs_no_more_than_the_note() {
    let mut s = Session::default();
    let rate = 44_100;
    let frames = rate * 4;
    let x: Vec<f32> = (0..frames).map(|i| if i < rate { 0.3 * (i as f32 * 0.05).sin() } else { 0.0 }).collect();
    s.pool.insert(SourceId(7), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: rate as u32, channels: vec![x.clone(), x] })));
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, None);
    let id = s.new_clip_id();
    let tr = s.track_mut(t).expect("track");
    tr.playlist_mut().unwrap().clips.push(Clip::audio(id, "c", SourceId(7), 0, 0, frames as i64));
    let mut ins = Insert::new("open_sat");
    for (k, v) in [("b1on", 1.0), ("b1drive", 43.0), ("b1freq", 436.0), ("b1type", 1.0)] {
        ins.params.insert(k.into(), v);
    }
    tr.mixer.inserts[0] = Some(ins);
    let time = |r: Range| {
        let t0 = std::time::Instant::now();
        openpt_mix::render_range(&s, r, 1024);
        t0.elapsed().as_secs_f64()
    };
    let note = time(Range::new(0, rate as i64));
    let after = time(Range::new(0, frames as i64)) - note;
    assert!(after < note * 3.0 * 3.0, "1 s of note {note:.3} s, 3 s of silence after it {after:.3} s");
}
