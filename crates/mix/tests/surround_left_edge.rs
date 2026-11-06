#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openpt_audio_io::AudioBuffer;
use openpt_mix::{monitor::Monitor, render_range};
use openpt_model::{AtmosMonitor, AtmosSettings, ChannelFormat, Clip, Route, Session, SourceAudio, SourceId, SurroundPan, TrackKind};
use openpt_time::Range;
use std::sync::Arc;

fn render(fmt: ChannelFormat, y: f32) -> (Vec<Vec<f32>>, f32, f32) {
    let mut s = Session::default();
    s.outputs[0].format = ChannelFormat::Atmos714;
    let n = 48_000usize;
    let mut st = 12345u32;
    let x: Vec<f32> = (0..n)
        .map(|_| {
            st = st.wrapping_mul(1664525).wrapping_add(1013904223);
            (st >> 8) as f32 / 16_777_216.0 - 0.5
        })
        .collect();
    let ch = vec![x; fmt.channels()];
    s.pool.insert(SourceId(5), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: ch })));
    let t = s.add_track(TrackKind::Audio, fmt, Some("T"));
    let c = s.new_clip_id();
    s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(c, "n", SourceId(5), 0, 0, n as i64));
    s.track_mut(t).unwrap().mixer.output = Route::Main;
    s.track_mut(t).unwrap().mixer.surround = Some(SurroundPan { x: -1.0, y, ..SurroundPan::default() });
    let speakers = render_range(&s, Range::new(0, n as i64), 512);
    let mut ears = speakers.clone();
    let set = AtmosSettings { monitor: AtmosMonitor::Binaural, ..AtmosSettings::default() };
    Monitor::new(&set, ChannelFormat::Atmos714, 48_000).process(&mut ears, n);
    let keep = (-std::f32::consts::TAU * 800.0 / 48_000.0f32).exp();
    let db = |c: &[f32]| {
        let mut state = 0.0f32;
        10.0 * c
            .iter()
            .map(|v| {
                state = *v + keep * (state - *v);
                state * state
            })
            .sum::<f32>()
            .log10()
    };
    (speakers, db(&ears[0]), db(&ears[1]))
}

#[test]
fn a_puck_down_the_left_edge_stays_left_at_a_steady_level() {
    for fmt in [ChannelFormat::Mono, ChannelFormat::Stereo] {
        let mut levels = Vec::new();
        for y in [1.0f32, 0.5, 0.0, -0.5, -1.0] {
            let (speakers, l, r) = render(fmt, y);
            let spk = ChannelFormat::Atmos714.speakers();
            for (c, sp) in speakers.iter().zip(spk) {
                if sp.azimuth() > 0.0 {
                    assert!(c.iter().all(|v| v.abs() < 1e-6), "{fmt:?} y {y}: {sp:?} carries signal");
                }
            }
            assert!(l > r + 1.5, "{fmt:?} y {y}: L {l} R {r}");
            levels.push(l);
        }
        let (lo, hi) = levels.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!(hi - lo < 4.0, "{fmt:?}: left-ear level swings {lo}..{hi}");
    }
}
