use super::more_util::replace_clip;
use super::*;
use crate::cmd;
use openpt_model::{ChannelFormat, ClipContent, ClipId, Session, SourceId, TrackKind};
use openpt_time::Samples;
use serde_json::json;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "clip.expand_channels",
            "Expand Channels to New Tracks",
            ["Clip"],
            None,
            "{clips?} — each channel of a multichannel audio clip goes onto a new mono track below, named after the file's iXML track names",
            has_selection,
            expand_channels
        ),
        cmd!(
            query "clip.field_matches",
            "Field Recorder Matches",
            [],
            None,
            "{clip: id} → {matches: [{source, name, channel, label}]} — files (and their channels) whose time stamp covers the clip's start",
            always,
            matches
        ),
        cmd!(
            "clip.field_replace",
            "Replace with Field Recorder Channel",
            [],
            None,
            "{clip: id, source: id, channel?: 0-based} — the clip plays the same moment from another file or channel",
            always,
            replace
        ),
    ]
}

fn channel_source(s: &mut Session, id: &str, src: SourceId, ch: usize) -> Result<SourceId> {
    let Some(x) = s.source(src).cloned() else { return Err(bad(id, "no such audio file")) };
    if x.channels <= 1 {
        return Ok(src);
    }
    let label = x.field.as_ref().and_then(|f| f.tracks.get(ch)).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| format!("{}", ch + 1));
    let name = format!("{}.{}", x.name, label);
    if let Some(found) = s.sources.iter().find(|y| y.name == name && y.channels == 1 && y.time_reference == x.time_reference) {
        return Ok(found.id);
    }
    let audio = s.pool.get(src).cloned().ok_or_else(|| bad(id, "the file's audio is not loaded"))?;
    let data = audio.channel(ch).map(<[f32]>::to_vec).ok_or_else(|| bad(id, "no such channel"))?;
    let buf = openpt_audio_io::AudioBuffer { sample_rate: audio.sample_rate(), channels: vec![data] };
    let new = crate::io::add_source(s, &name, buf, None, x.format);
    if let Some(y) = s.sources.iter_mut().find(|y| y.id == new) {
        y.time_reference = x.time_reference;
        y.field = x.field.map(|mut f| {
            f.tracks = vec![label];
            f
        });
    }
    Ok(new)
}

fn expand_channels(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.expand_channels";
    let ids = clip_ids_param(e, p);
    let s = e.session_mut();
    let mut tracks = Vec::new();
    for cid in ids {
        let Some((t, c)) = s.find_clip(cid).map(|(t, c)| (t, c.clone())) else { continue };
        let ClipContent::Audio { source, offset } = c.content else { continue };
        let Some(src) = s.source(source).cloned() else { continue };
        if src.channels <= 1 {
            continue;
        }
        let base = s.track(t).map(|x| x.name.clone()).unwrap_or_default();
        let mut at = s.track_index(t).unwrap_or(s.tracks.len());
        for ch in 0..usize::from(src.channels) {
            let mono = channel_source(s, id, source, ch)?;
            let name = src.field.as_ref().and_then(|f| f.tracks.get(ch)).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| format!("{base} {}", ch + 1));
            let nt = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some(&name));
            if let Some(i) = s.track_index(nt) {
                let tr = s.tracks.remove(i);
                at += 1;
                s.tracks.insert(at.min(s.tracks.len()), tr);
            }
            let mut nc = c.clone();
            nc.id = s.new_clip_id();
            nc.content = ClipContent::Audio { source: mono, offset };
            nc.name = s.source(mono).map_or_else(|| c.name.clone(), |x| x.name.clone());
            nc.group = None;
            nc.loop_id = None;
            nc.warp = None;
            crate::edit::place_clip(s, nt, nc);
            tracks.push(nt);
        }
    }
    if tracks.is_empty() {
        return Err(bad(id, "select a multichannel audio clip"));
    }
    Ok(json!({"tracks": tracks}))
}

fn clip_stamp(s: &Session, c: &openpt_model::Clip) -> Option<(u64, Samples)> {
    let ClipContent::Audio { source, offset } = c.content else { return None };
    let x = s.source(source)?;
    (x.time_reference > 0).then(|| (x.time_reference.saturating_add(u64::try_from(offset).unwrap_or(0)), c.length))
}

fn matches(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.field_matches";
    let cid = p.get("clip").and_then(Value::as_u64).map(ClipId).ok_or_else(|| bad(id, "`clip` id required"))?;
    let s = e.session();
    let c = s.find_clip(cid).map(|(_, c)| c.clone()).ok_or_else(|| bad(id, "no such clip"))?;
    let Some((tc, _)) = clip_stamp(s, &c) else { return Ok(json!({"matches": []})) };
    let own = match c.content {
        ClipContent::Audio { source, .. } => Some(source),
        _ => None,
    };
    let mut out = Vec::new();
    for x in &s.sources {
        if x.time_reference == 0 || x.time_reference > tc || tc >= x.time_reference.saturating_add(x.frames) {
            continue;
        }
        let derived = x.channels == 1 && s.sources.iter().any(|y| y.channels > 1 && x.name.starts_with(&format!("{}.", y.name)));
        if derived {
            continue;
        }
        let st = x.field.as_ref().map(openpt_audio_io::FieldInfo::scene_take).unwrap_or_default();
        for ch in 0..usize::from(x.channels.max(1)) {
            if Some(x.id) == own && x.channels == 1 {
                continue;
            }
            let track = x.field.as_ref().and_then(|f| f.tracks.get(ch)).filter(|n| !n.is_empty()).cloned();
            let label = match (&track, x.channels > 1) {
                (Some(t), _) => format!("{} — {t}", x.name),
                (None, true) => format!("{} — ch {}", x.name, ch + 1),
                (None, false) => x.name.clone(),
            };
            let label = if st.is_empty() { label } else { format!("{label} ({st})") };
            out.push(json!({"source": x.id, "name": x.name, "channel": ch, "label": label}));
        }
    }
    Ok(json!({"matches": out}))
}

fn replace(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.field_replace";
    let cid = p.get("clip").and_then(Value::as_u64).map(ClipId).ok_or_else(|| bad(id, "`clip` id required"))?;
    let src = p.get("source").and_then(Value::as_u64).map(SourceId).ok_or_else(|| bad(id, "`source` id required"))?;
    let ch = p.get("channel").and_then(Value::as_u64).map_or(0, |c| usize::try_from(c).unwrap_or(0));
    let s = e.session_mut();
    let (t, c) = s.find_clip(cid).map(|(t, c)| (t, c.clone())).ok_or_else(|| bad(id, "no such clip"))?;
    let (tc, len) = clip_stamp(s, &c).ok_or_else(|| bad(id, "the clip's file has no time stamp"))?;
    let x = s.source(src).cloned().ok_or_else(|| bad(id, "no such audio file"))?;
    if x.time_reference == 0 || x.time_reference > tc {
        return Err(bad(id, "that file does not cover the clip's start"));
    }
    let offset = Samples::try_from(tc - x.time_reference).unwrap_or(0);
    let track_channels = s.track(t).map_or(1, |tr| tr.channels());
    let use_src = if x.channels > 1 && track_channels == 1 { channel_source(s, id, src, ch)? } else { src };
    let avail = s.source(use_src).map_or(0, |y| Samples::try_from(y.frames).unwrap_or(0)) - offset;
    let mut nc = c.clone();
    nc.content = ClipContent::Audio { source: use_src, offset };
    nc.length = len.min(avail.max(1));
    nc.name = s.source(use_src).map_or(c.name.clone(), |y| y.name.clone());
    nc.warp = None;
    replace_clip(s, t, cid, nc);
    Ok(json!({"clip": cid, "source": use_src, "offset": offset}))
}

#[cfg(test)]
mod tests {
    use crate::Engine;
    use openpt_model::{ClipContent, ClipId, SourceId};
    use serde_json::json;

    fn session() -> (Engine, u64, SourceId, SourceId) {
        let mut e = Engine::default();
        let rate = e.session().sample_rate.hz();
        let n = rate as usize;
        let s = e.session_mut();
        let poly = crate::io::add_source(
            s,
            "Sc12_T3",
            openpt_audio_io::AudioBuffer { sample_rate: rate, channels: vec![vec![0.1; n], vec![0.2; n], vec![0.3; n]] },
            None,
            openpt_audio_io::FileFormat::Wav,
        );
        let boom = crate::io::add_source(
            s,
            "Boom",
            openpt_audio_io::AudioBuffer { sample_rate: rate, channels: vec![vec![0.5; 2 * n], vec![0.6; 2 * n]] },
            None,
            openpt_audio_io::FileFormat::Wav,
        );
        for x in &mut s.sources {
            if x.id == poly {
                x.time_reference = 10 * u64::from(rate);
                x.field = Some(openpt_audio_io::FieldInfo { scene: "12".into(), take: "3".into(), tracks: vec!["Mix".into(), "Boom".into(), "Lav".into()], ..Default::default() });
            } else {
                x.time_reference = 9 * u64::from(rate);
            }
        }
        let t = e.execute("track.new", &json!({"format": "mono"})).unwrap()["tracks"][0].as_u64().unwrap();
        let clip = e.execute("clip.place_source", &json!({"source": poly.0, "track": t, "at": 0})).unwrap()["clip"].as_u64().unwrap();
        (e, clip, poly, boom)
    }

    #[test]
    fn expand_channels_makes_one_mono_track_per_ixml_track() {
        let (mut e, clip, _, _) = session();
        let r = e.execute("clip.expand_channels", &json!({"clips": [clip]})).unwrap();
        assert_eq!(r["tracks"].as_array().unwrap().len(), 3);
        let s = e.session();
        for (name, level) in [("Mix", 0.1f32), ("Boom", 0.2), ("Lav", 0.3)] {
            let tr = s.track_by_name(name).unwrap_or_else(|| panic!("no track {name}"));
            let c = &tr.clips()[0];
            let ClipContent::Audio { source, .. } = c.content else { panic!() };
            let audio = s.pool.get(source).unwrap();
            assert_eq!((audio.num_channels(), audio.sample(0, 10)), (1, level), "{name}");
        }
    }

    #[test]
    fn a_clip_swaps_to_the_same_moment_of_another_file() {
        let (mut e, clip, _, boom) = session();
        let m = e.execute("clip.field_matches", &json!({"clip": clip})).unwrap();
        let labels: Vec<&str> = m["matches"].as_array().unwrap().iter().map(|x| x["label"].as_str().unwrap()).collect();
        assert!(labels.iter().any(|l| l.contains("Boom") && l.contains("ch 2")), "{labels:?}");
        assert!(labels.iter().any(|l| l.contains("Lav") && l.contains("Sc 12 Tk 3")), "{labels:?}");
        e.execute("clip.field_replace", &json!({"clip": clip, "source": boom.0, "channel": 1})).unwrap();
        let s = e.session();
        let c = s.find_clip(ClipId(clip)).unwrap().1;
        let ClipContent::Audio { source, offset } = c.content else { panic!() };
        assert_eq!(offset, i64::from(s.sample_rate.hz()), "one second into the boom file");
        assert_eq!(s.pool.get(source).unwrap().sample(0, 0), 0.6, "its second channel, on a mono track");
    }
}
