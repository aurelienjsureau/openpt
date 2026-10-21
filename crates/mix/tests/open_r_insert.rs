use openpt_audio_io::AudioBuffer;
use openpt_model::{ChannelFormat, Clip, Insert, Session, SourceAudio, SourceId, TrackKind};
use openpt_time::Range;
use std::sync::Arc;

fn render(format: ChannelFormat) -> Vec<Vec<f32>> {
    let mut s = Session::default();
    let frames = 48_000;
    let chans = format.channels();
    let click: Vec<f32> = (0..frames).map(|i| if i % 12_000 < 200 { ((i as f32) * 0.3).sin() * 0.5 } else { 0.0 }).collect();
    s.pool.insert(SourceId(7), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![click; chans] })));
    let t = s.add_track(TrackKind::Audio, format, None);
    let id = s.new_clip_id();
    let tr = s.track_mut(t).expect("track");
    if let Some(p) = tr.playlist_mut() {
        p.clips.push(Clip::audio(id, "c", SourceId(7), 0, 0, frames as i64));
    }
    let info = openpt_dsp::plugin_info("open_r").expect("registered");
    let mut ins = Insert::new("open_r");
    for p in info.params {
        ins.params.insert(p.id.to_string(), p.default);
    }
    tr.mixer.inserts[0] = Some(ins);
    openpt_mix::render_range(&s, Range::new(0, frames as i64), 512)
}

#[test]
fn a_clip_through_open_r_comes_out_as_reverb() {
    for format in [ChannelFormat::Mono, ChannelFormat::Stereo] {
        let out = render(format);
        let gap: f32 = out.iter().map(|c| c[3_000..11_000].iter().map(|v| v * v).sum::<f32>()).sum();
        assert!(gap > 1.0e-4, "{format:?}: reverb energy {gap}");
    }
}

#[test]
fn an_aux_with_open_r_returns_the_reverb() {
    use openpt_model::Route;
    let mut s = Session::default();
    let frames = 48_000;
    let click: Vec<f32> = (0..frames).map(|i| if i % 12_000 < 200 { ((i as f32) * 0.3).sin() * 0.5 } else { 0.0 }).collect();
    s.pool.insert(SourceId(7), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![click.clone(), click] })));
    let bus = s.add_bus("Verb", ChannelFormat::Stereo);
    let src = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Src"));
    let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Verb"));
    let id = s.new_clip_id();
    s.track_mut(src).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(id, "c", SourceId(7), 0, 0, frames as i64));
    let mut send = openpt_model::SendSlot::new(Route::Bus(bus));
    send.level_db = 0.0;
    s.track_mut(src).unwrap().mixer.sends[0] = Some(send);
    s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
    let info = openpt_dsp::plugin_info("open_r").unwrap();
    let mut ins = Insert::new("open_r");
    for p in info.params {
        ins.params.insert(p.id.to_string(), p.default);
    }
    s.track_mut(aux).unwrap().mixer.inserts[0] = Some(ins);
    let out = openpt_mix::render_range(&s, Range::new(0, frames as i64), 512);
    let all: f32 = out.iter().map(|c| c.iter().map(|v| v * v).sum::<f32>()).sum();
    let gap: f32 = out.iter().map(|c| c[3_000..11_000].iter().map(|v| v * v).sum::<f32>()).sum();
    assert!(all > 1.0e-3 && gap > 1.0e-4, "total {all}, reverb {gap}");
}

#[test]
fn soloing_the_source_keeps_its_aux_heard() {
    use openpt_model::Route;
    for safe in [false, true] {
        let mut s = Session::default();
        let frames = 24_000;
        let tone: Vec<f32> = (0..frames).map(|i| ((i as f32) * 0.05).sin() * 0.3).collect();
        s.pool.insert(SourceId(7), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![tone.clone(), tone] })));
        let bus = s.add_bus("Verb", ChannelFormat::Stereo);
        let src = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Src"));
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Verb"));
        let id = s.new_clip_id();
        s.track_mut(src).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(id, "c", SourceId(7), 0, 0, frames as i64));
        let mut send = openpt_model::SendSlot::new(Route::Bus(bus));
        send.level_db = 0.0;
        s.track_mut(src).unwrap().mixer.sends[0] = Some(send);
        s.track_mut(src).unwrap().mixer.output = Route::None;
        s.track_mut(src).unwrap().mixer.solo = true;
        s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
        s.track_mut(aux).unwrap().mixer.solo_safe = safe;
        let out = openpt_mix::render_range(&s, Range::new(0, frames as i64), 512);
        let e: f32 = out.iter().map(|c| c.iter().map(|v| v * v).sum::<f32>()).sum();
        assert!(e > 1.0, "safe {safe}: the aux is silent ({e})");
    }
}

#[test]
fn an_aux_returning_to_an_earlier_aux_is_heard_in_solo() {
    use openpt_model::Route;
    let run = |aux_muted: bool| {
        let mut s = Session::default();
        let frames = 24_000;
        let tone: Vec<f32> = (0..frames).map(|i| if i < 2_000 { ((i as f32) * 0.05).sin() * 0.3 } else { 0.0 }).collect();
        s.pool.insert(SourceId(7), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![tone.clone(), tone] })));
        let snare_bus = s.add_bus("Snare", ChannelFormat::Stereo);
        let drum_bus = s.add_bus("Drums", ChannelFormat::Stereo);
        let verb_bus = s.add_bus("Verb", ChannelFormat::Stereo);
        let snare = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Snare Top"));
        let drums = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("DRUM BUS"));
        let src = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Src"));
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Aux 2"));
        for (t, i, o) in [(snare, snare_bus, Route::Bus(drum_bus)), (drums, drum_bus, Route::Main), (aux, verb_bus, Route::Bus(snare_bus))] {
            let m = &mut s.track_mut(t).unwrap().mixer;
            m.input = Route::Bus(i);
            m.output = o;
            m.solo_safe = t != aux;
        }
        let id = s.new_clip_id();
        s.track_mut(src).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(id, "c", SourceId(7), 0, 0, frames as i64));
        let mut send = openpt_model::SendSlot::new(Route::Bus(verb_bus));
        send.level_db = 0.0;
        let m = &mut s.track_mut(src).unwrap().mixer;
        m.sends[0] = Some(send);
        m.output = Route::Bus(snare_bus);
        m.solo = true;
        let mut ins = Insert::new("open_r");
        for p in openpt_dsp::plugin_info("open_r").unwrap().params {
            ins.params.insert(p.id.to_string(), p.default);
        }
        let m = &mut s.track_mut(aux).unwrap().mixer;
        m.inserts[0] = Some(ins);
        m.mute = aux_muted;
        let out = openpt_mix::render_range(&s, Range::new(0, frames as i64), 512);
        out.iter().map(|c| c[6_000..].iter().map(|v| v * v).sum::<f32>()).sum::<f32>()
    };
    let (with, without) = (run(false), run(true));
    assert!(with > without + 1.0e-3, "the reverb tail: {with} with the aux, {without} muted");
}
