use super::more_util::{clip_db, set_clip_env};
use super::*;
use crate::cmd;
use serde_json::json;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("clip.edit_lock", "Edit Lock/Unlock", ["Clip"], Some("Cmd+L"), "{clips?}", has_selection, |e, p| flag(
            e,
            p,
            |c| c.edit_locked,
            |c, v| c.edit_locked = v
        )),
        cmd!("clip.time_lock", "Time Lock/Unlock", ["Clip"], Some("Ctrl+Alt+L"), "{clips?}", has_selection, |e, p| flag(
            e,
            p,
            |c| c.time_locked,
            |c, v| c.time_locked = v
        )),
        cmd!("clip.send_to_back", "Send to Back", ["Clip"], Some("Alt+Shift+B"), "{clips?}", has_selection, |e, p| layer(e, p, false)),
        cmd!("clip.bring_to_front", "Bring to Front", ["Clip"], Some("Alt+Shift+F"), "{clips?}", has_selection, |e, p| layer(e, p, true)),
        cmd!("clip.rating", "Rating", ["Clip", "Rating"], Some("Cmd+Alt+Ctrl+0-5"), "{clips?, rating: 0..5}", has_selection, |e, p| {
            let ids = clip_ids_param(e, p);
            let r = i64_or(p, "rating", 0).clamp(0, 5) as u8;
            let s = e.session_mut();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.rating = r
                }
            }
            Ok(json!({"rating": r}))
        }),
        cmd!("clip.group", "Group", ["Clip"], Some("Cmd+Alt+G"), "{clips?}", has_selection, |e, p| {
            let ids = clip_ids_param(e, p);
            if ids.len() < 2 {
                return Err(bad("clip.group", "select two or more clips"));
            }
            let s = e.session_mut();
            let g = s.alloc();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.group = Some(g)
                }
            }
            Ok(json!({"group": g}))
        }),
        cmd!("clip.ungroup", "Ungroup", ["Clip"], Some("Cmd+Alt+U"), "{clips?}", has_selection, |e, p| {
            let ids = clip_ids_param(e, p);
            let s = e.session_mut();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.group = None
                }
            }
            Ok(json!({}))
        }),
        cmd!("clip.ungroup_all", "Ungroup All", ["Clip"], None, "{}", always, |e, _| {
            for t in &mut e.session_mut().tracks {
                for pl in &mut t.playlists {
                    for c in &mut pl.clips {
                        c.group = None
                    }
                }
            }
            Ok(json!({}))
        }),
        cmd!("clip.loop", "Loop...", ["Clip"], Some("Cmd+Alt+L"), "{clips?, count?: n, length?: samples}", has_selection, loop_clips),
        cmd!("clip.rename", "Rename...", ["Clip"], Some("Cmd+Alt+Shift+R"), "{clip?, name}", has_selection, rename),
        cmd!("clip.gain", "Clip Gain", [], None, "{clips?, db | delta_db}", has_selection, gain),
        cmd!(
            "clip.gain_point",
            "Clip Gain Point",
            [],
            None,
            "{clip, at: samples from the clip start, db, from?: the point being moved}",
            always,
            gain_point
        ),
        cmd!(
            "clip.gain_trim_range",
            "Trim Clip Gain",
            [],
            None,
            "{clip, start, end: samples from the clip start, delta: dB}",
            always,
            gain_trim_range
        ),
        cmd!("clip.gain_point_remove", "Remove Clip Gain Point", [], None, "{clip, at: samples from the clip start}", always, gain_point_remove),
        cmd!("clip.gain_nudge_up", "Nudge Clip Gain Up", [], Some("Ctrl+Shift+Up"), "{clips?}", has_selection, |e, p| nudge_gain(e, p, 1.0)),
        cmd!("clip.gain_nudge_down", "Nudge Clip Gain Down", [], Some("Ctrl+Shift+Down"), "{clips?}", has_selection, |e, p| nudge_gain(e, p, -1.0)),
        cmd!("clip.gain_render", "Render", ["Clip", "Clip Gain"], None, "{clips?}", has_selection, |e, p| {
            let ids = clip_ids_param(e, p);
            let mut n = 0;
            for id in ids {
                if crate::io::render_clip_gain(e, "clip.gain_render", id)? {
                    n += 1
                }
            }
            Ok(json!({"rendered": n}))
        }),
        cmd!("clip.gain_bypass", "Bypass", ["Clip", "Clip Gain"], None, "{clips?}", has_selection, |e, p| {
            let ids = clip_ids_param(e, p);
            let s = e.session_mut();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.gain_db = 0.0;
                    c.gain_env.clear()
                }
            }
            Ok(json!({}))
        }),
        cmd!("clip.identify_sync_point", "Identify Sync Point", ["Clip"], Some("Cmd+,"), "{clip?, at?}", has_selection, |e, p| {
            let at = position_param(e, "clip.identify_sync_point", p, "at")?.unwrap_or(e.session().edit.selection.start);
            let ids = clip_ids_param(e, p);
            let s = e.session_mut();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.sync_point = (at - c.start).clamp(0, c.length)
                }
            }
            Ok(json!({}))
        }),
        cmd!("clip.quantize_to_grid", "Quantize to Grid", ["Clip"], Some("Cmd+0"), "{clips?}", has_selection, quantize_to_grid),
        cmd!("clip.color", "Clip Color", [], None, "{clips?, color: [r,g,b] | null}", has_selection, |e, p| {
            let ids = clip_ids_param(e, p);
            let col = p.get("color").and_then(Value::as_array).map(|a| {
                let g = |i: usize| a.get(i).and_then(Value::as_u64).unwrap_or(0).min(255) as u8;
                [g(0), g(1), g(2)]
            });
            let s = e.session_mut();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.color = col
                }
            }
            Ok(json!({}))
        }),
        cmd!(
            "clip.elastic_properties",
            "Elastic Properties",
            ["Clip"],
            Some("Alt+5"),
            "{clips?, ratio?: 1.0} — on Elastic Audio tracks (polyphonic/rhythmic/monophonic/x-form) the stretch is rendered pitch-preserving; elsewhere it is varispeed",
            has_selection,
            elastic
        ),
        cmd!("clip.remove_warp", "Remove Warp", ["Clip"], None, "{clips?}", has_selection, remove_warp),
        cmd!(
            "clip.place_source",
            "Place Audio File",
            [],
            None,
            "{source: id, track?: id|name (default: a new track), at?: position}",
            always,
            |e, p| {
                let src = p
                    .get("source")
                    .and_then(Value::as_u64)
                    .map(openpt_model::SourceId)
                    .ok_or_else(|| bad("clip.place_source", "`source` id required"))?;
                let at = position_param(e, "clip.place_source", p, "at")?.unwrap_or(e.session().edit.selection.start).max(0);
                let track = track_param(e, "clip.place_source", p, "track")?;
                let (name, frames, chans) = e
                    .session()
                    .source(src)
                    .map(|x| (x.name.clone(), i64::try_from(x.frames).unwrap_or(0), usize::from(x.channels)))
                    .ok_or_else(|| bad("clip.place_source", "no such audio file"))?;
                let s = e.session_mut();
                let t = match track {
                    Some(t) if s.track(t).is_some_and(|x| x.kind == openpt_model::TrackKind::Audio) => t,
                    Some(_) => return Err(bad("clip.place_source", "audio files go on audio tracks")),
                    None => s.add_track(openpt_model::TrackKind::Audio, openpt_model::ChannelFormat::for_channels(chans.min(2)), Some(&name)),
                };
                let (sh, len) = (s.edit.edit_mode == openpt_model::EditMode::Shuffle, frames.max(1));
                let at = if sh { crate::edit::shuffle_slot(s, t, at, &[]) } else { at };
                if sh {
                    crate::edit::make_room(s, t, at, len).map_err(|why| bad("clip.place_source", why))?;
                }
                let id = s.new_clip_id();
                crate::edit::place_clip(s, t, openpt_model::Clip::audio(id, name, src, 0, at, len));
                Ok(json!({"clip": id, "track": t}))
            }
        ),
        cmd!(
            "clip.place_sources",
            "Place Audio Files",
            [],
            None,
            "{sources: [id], track: id|name, at?: position} — as many mono files as the track has channels become one multichannel file; a multichannel file on a mono track is split over it and the mono tracks below; otherwise one file per track from `track` down",
            always,
            place_sources
        ),
        cmd!("clip.capture", "Capture...", ["Clip"], Some("Cmd+R"), "{name?}", has_range, |e, p| {
            let name = str_param(p, "name").unwrap_or("Captured").to_string();
            let ids = clip_ids_param(e, p);
            let s = e.session_mut();
            for id in &ids {
                if let Some(c) = s.find_clip_mut(*id) {
                    c.name = name.clone()
                }
            }
            Ok(json!({"clips": ids.len()}))
        }),
    ]
}

fn elastic(e: &mut Engine, p: &Value) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let ratio = f64_or(p, "ratio", 1.0).clamp(0.05, 20.0);
    let s = e.session_mut();
    let mut rendered = 0;
    for id in &ids {
        let Some((t, c)) = s.find_clip(*id).map(|(t, c)| (t, c.clone())) else { continue };
        if let Some(algo) = s.track(t).and_then(|x| x.elastic.clone()).filter(|a| a != "none")
            && c.is_audio()
            && warp_original(s, *id).is_none()
        {
            super::warp::stretch(s, "clip.elastic_properties", *id, ratio, &algo)?;
            rendered += 1;
            continue;
        }
        let elastic = s.track(t).and_then(|x| x.elastic.clone()).filter(|a| a != "varispeed");
        let key = |k: &str| format!("elastic.{}.{k}", id.0);
        let orig = warp_original(s, *id);
        let mut base = c.clone();
        if let Some((src, off, len)) = orig {
            base.content = openpt_model::ClipContent::Audio { source: src, offset: off };
            base.length = len.max(1);
            base.stretch = 1.0;
        }
        if elastic.is_none() || !base.is_audio() {
            if let Some(cm) = s.find_clip_mut(*id) {
                cm.stretch = ratio;
            }
            continue;
        }
        let out_len = openpt_time::to_samples(base.length as f64 * ratio).max(1);
        let Some(src) = super::more_util::render_stretched(s, "clip.elastic_properties", &base, out_len)? else { continue };
        if orig.is_none()
            && let openpt_model::ClipContent::Audio { source, offset } = base.content
        {
            s.edit.values.insert(key("src"), source.0 as f64);
            s.edit.values.insert(key("off"), offset as f64);
            s.edit.values.insert(key("len"), base.length as f64);
        }
        s.edit.values.insert(key("ratio"), ratio);
        let mut nc = base;
        nc.content = openpt_model::ClipContent::Audio { source: src, offset: 0 };
        nc.length = out_len;
        nc.stretch = 1.0;
        super::more_util::replace_clip(s, t, *id, nc);
        rendered += 1;
    }
    Ok(json!({"ratio": ratio, "rendered": rendered}))
}

fn common_stem(names: &[String]) -> String {
    let first = names.first().map(String::as_str).unwrap_or("Audio");
    let prefix_len = names.iter().skip(1).fold(first.len(), |n, other| first.bytes().zip(other.bytes()).take(n).take_while(|(a, b)| a == b).count());
    let stem = first.get(..prefix_len).unwrap_or(first).trim_end_matches(['.', '_', '-', ' ']);
    if stem.is_empty() { first.to_string() } else { stem.to_string() }
}

fn place_on(
    s: &mut openpt_model::Session,
    t: TrackId,
    src: openpt_model::SourceId,
    name: String,
    frames: Samples,
    at: Samples,
) -> Result<openpt_model::ClipId> {
    let (sh, len) = (s.edit.edit_mode == openpt_model::EditMode::Shuffle, frames.max(1));
    let at = if sh { crate::edit::shuffle_slot(s, t, at, &[]) } else { at };
    if sh {
        crate::edit::make_room(s, t, at, len).map_err(|why| bad("clip.place_sources", why))?;
    }
    let id = s.new_clip_id();
    crate::edit::place_clip(s, t, openpt_model::Clip::audio(id, name, src, 0, at, len));
    Ok(id)
}

fn place_sources(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.place_sources";
    let ids: Vec<openpt_model::SourceId> = p
        .get("sources")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_u64).map(openpt_model::SourceId).collect())
        .unwrap_or_default();
    if ids.is_empty() {
        return Err(bad(id, "`sources` required"));
    }
    let at = position_param(e, id, p, "at")?.unwrap_or(e.session().edit.selection.start).max(0);
    let track = track_param(e, id, p, "track")?.ok_or_else(|| bad(id, "`track` required"))?;
    let s = e.session_mut();
    let mut files = Vec::new();
    for sid in &ids {
        let src = s.source(*sid).ok_or_else(|| bad(id, "no such audio file"))?;
        let audio = s.pool.get(*sid).cloned().ok_or_else(|| bad(id, format!("{} is not loaded", src.name)))?;
        files.push((src.name.clone(), src.format, audio));
    }
    let audio_tracks: Vec<TrackId> = s.tracks.iter().filter(|t| t.kind == openpt_model::TrackKind::Audio).map(|t| t.id).collect();
    let from = audio_tracks.iter().position(|t| *t == track).ok_or_else(|| bad(id, "audio files go on audio tracks"))?;
    let widths: Vec<usize> = audio_tracks.iter().map(|t| s.track(*t).map_or(0, |x| x.format.channels())).collect();
    let width = |i: usize| widths.get(i).copied().unwrap_or(0);
    let mut placed = Vec::new();
    let all_mono = files.iter().all(|f| f.2.buffer.num_channels() == 1);
    if files.len() > 1 && all_mono && width(from) == files.len() {
        let frames = files.iter().map(|f| f.2.frames()).max().unwrap_or(0);
        let channels = files
            .iter()
            .map(|f| {
                f.2.buffer
                    .channels
                    .first()
                    .map(|c| {
                        let mut c = c.clone();
                        c.resize(frames, 0.0);
                        c
                    })
                    .unwrap_or_else(|| vec![0.0; frames])
            })
            .collect();
        let name = common_stem(&files.iter().map(|f| f.0.clone()).collect::<Vec<_>>());
        let buf = openpt_audio_io::AudioBuffer { sample_rate: s.sample_rate.hz(), channels };
        let src = crate::io::add_source(s, &name, buf, None, files[0].1);
        placed.push(place_on(s, track, src, name, i64::try_from(frames).unwrap_or(i64::MAX), at)?);
    } else if files.len() == 1 && files[0].2.buffer.num_channels() > 1 && width(from) == 1 {
        let (name, format, audio) = &files[0];
        let n = audio.buffer.num_channels();
        let targets: Vec<TrackId> =
            (from..audio_tracks.len()).take_while(|i| width(*i) == 1).take(n).filter_map(|i| audio_tracks.get(i).copied()).collect();
        if targets.len() < n {
            return Err(bad(id, format!("{name} has {n} channels: it needs {n} mono tracks from here down")));
        }
        let fmt = openpt_model::ChannelFormat::for_channels(n);
        for (k, t) in targets.into_iter().enumerate() {
            let ch_name = format!("{name}.{}", fmt.channel_label(k));
            let buf = openpt_audio_io::AudioBuffer {
                sample_rate: s.sample_rate.hz(),
                channels: vec![audio.buffer.channels.get(k).cloned().unwrap_or_default()],
            };
            let src = crate::io::add_source(s, &ch_name, buf, None, *format);
            placed.push(place_on(s, t, src, ch_name, i64::try_from(audio.frames()).unwrap_or(i64::MAX), at)?);
        }
    } else {
        for (k, (name, _, audio)) in files.iter().enumerate() {
            let t = match audio_tracks.get(from + k) {
                Some(t) => *t,
                None => s.add_track(
                    openpt_model::TrackKind::Audio,
                    openpt_model::ChannelFormat::for_channels(audio.buffer.num_channels().min(2)),
                    Some(name),
                ),
            };
            let src = ids[k];
            placed.push(place_on(s, t, src, name.clone(), i64::try_from(audio.frames()).unwrap_or(i64::MAX), at)?);
        }
    }
    Ok(json!({"clips": placed}))
}

fn remove_warp(e: &mut Engine, p: &Value) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let s = e.session_mut();
    let mut n = 0;
    for id in &ids {
        if super::warp::unwarp(s, *id) {
            n += 1;
            continue;
        }
        let key = |k: &str| format!("elastic.{}.{k}", id.0);
        let orig = warp_original(s, *id);
        if let Some(c) = s.find_clip_mut(*id) {
            c.stretch = 1.0;
            if let Some((src, off, len)) = orig {
                c.content = openpt_model::ClipContent::Audio { source: src, offset: off };
                c.length = len.max(1);
                c.fade_out.len = c.fade_out.len.min(c.length);
                n += 1;
            }
        }
        for k in ["src", "off", "len", "ratio"] {
            s.edit.values.remove(&key(k));
        }
    }
    Ok(json!({"restored": n}))
}

fn warp_original(s: &openpt_model::Session, id: ClipId) -> Option<(openpt_model::SourceId, Samples, Samples)> {
    let v = |k: &str| s.edit.values.get(&format!("elastic.{}.{k}", id.0)).copied();
    let src = openpt_model::SourceId(v("src")? as u64);
    s.source(src)?;
    Some((src, v("off")? as i64, v("len")? as i64))
}

fn flag(e: &mut Engine, p: &Value, get: fn(&openpt_model::Clip) -> bool, set: fn(&mut openpt_model::Clip, bool)) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let s = e.session_mut();
    let v = p.get("value").and_then(Value::as_bool).unwrap_or_else(|| !ids.iter().all(|id| s.find_clip(*id).is_some_and(|(_, c)| get(c))));
    for id in &ids {
        if let Some(c) = s.find_clip_mut(*id) {
            set(c, v);
        }
    }
    Ok(json!({"value": v, "clips": ids.len()}))
}

fn layer(e: &mut Engine, p: &Value, front: bool) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let s = e.session_mut();
    for t in &mut s.tracks {
        if let Some(pl) = t.playlist_mut() {
            let (mut sel, mut rest): (Vec<_>, Vec<_>) = pl.clips.drain(..).partition(|c| ids.contains(&c.id));
            if front {
                rest.append(&mut sel);
                pl.clips = rest;
            } else {
                sel.append(&mut rest);
                pl.clips = sel;
            }
        }
    }
    Ok(json!({}))
}

fn loop_clips(e: &mut Engine, p: &Value) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let count = i64_or(p, "count", 4).clamp(1, 512);
    let total = position_param(e, "clip.loop", p, "length")?;
    let s = e.session_mut();
    let mut made = 0;
    for id in ids {
        let Some((tid, mut c)) = s.find_clip(id).map(|(t, c)| (t, c.clone())) else { continue };
        let reps = total.map_or(count - 1, |l| (l / c.length.max(1)).max(1) - 1).clamp(0, 1000);
        if reps > 0 && c.loop_id.is_none() {
            let lid = s.alloc();
            if let Some(src) = s.find_clip_mut(id) {
                src.loop_id = Some(lid);
            }
            c.loop_id = Some(lid);
        }
        let mut copies = Vec::new();
        for k in 1..=reps {
            let mut nc = c.clone();
            nc.start = 0;
            copies.push((c.start + c.length * k, nc));
        }
        for (at, nc) in copies {
            let _ = crate::edit::paste(s, tid, at, &[nc], c.length, false);
            made += 1;
        }
    }
    Ok(json!({"loops": made}))
}

fn rename(e: &mut Engine, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("clip.rename", "`name` required"))?.to_string();
    let ids = clip_ids_param(e, p);
    let s = e.session_mut();
    for id in &ids {
        if let Some(c) = s.find_clip_mut(*id) {
            c.name = name.clone();
        }
    }
    Ok(json!({"clips": ids.len()}))
}

fn gain_point(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.gain_point";
    let cid = clip_id(p, id)?;
    let at = p.get("at").and_then(Value::as_i64).ok_or_else(|| bad(id, "`at` required"))?;
    let db = p.get("db").and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(id, "`db` required"))? as f32;
    let from = p.get("from").and_then(Value::as_i64);
    let c = e.session_mut().find_clip_mut(cid).ok_or_else(|| bad(id, "no such clip"))?;
    let mut pts: Vec<(Samples, f32)> = if c.gain_env.is_empty() {
        vec![(0, c.gain_db), (c.length, c.gain_db)]
    } else {
        c.gain_env.iter().map(|&(o, _)| (o, clip_db(c, o))).collect()
    };
    let mut lo = 0;
    let mut hi = c.length;
    if let Some(f) = from
        && let Some(i) = pts.iter().position(|q| q.0 == f)
    {
        lo = if i == 0 { 0 } else { pts.get(i - 1).map_or(0, |q| q.0 + 1) };
        hi = pts.get(i + 1).map_or(c.length, |q| q.0 - 1);
        pts.remove(i);
    }
    let at = at.clamp(lo, hi.max(lo));
    pts.retain(|q| q.0 != at);
    pts.push((at, db.clamp(-144.0, 36.0)));
    pts.sort_by_key(|q| q.0);
    c.gain_db = 0.0;
    c.gain_env = pts;
    Ok(json!({"at": at}))
}

fn gain_trim_range(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.gain_trim_range";
    let cid = clip_id(p, id)?;
    let num = |k: &str| p.get(k).and_then(Value::as_i64).ok_or_else(|| bad(id, format!("`{k}` required")));
    let (start, end) = (num("start")?, num("end")?);
    let delta = p.get("delta").and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(id, "`delta` required"))? as f32;
    let c = e.session_mut().find_clip_mut(cid).ok_or_else(|| bad(id, "no such clip"))?;
    let (start, end) = (start.clamp(0, c.length), end.clamp(0, c.length));
    if end <= start {
        return Ok(json!({}));
    }
    let mut pts: Vec<(Samples, f32)> = if c.gain_env.is_empty() {
        vec![(0, c.gain_db), (c.length, c.gain_db)]
    } else {
        c.gain_env.iter().map(|&(o, _)| (o, clip_db(c, o))).collect()
    };
    let mut anchors = vec![(start, c.gain_db_at(start)), (end, c.gain_db_at(end))];
    if start > 0 {
        anchors.push((start - 1, c.gain_db_at(start - 1)));
    }
    if end < c.length {
        anchors.push((end + 1, c.gain_db_at(end + 1)));
    }
    for a in anchors {
        if !pts.iter().any(|q| q.0 == a.0) {
            pts.push(a);
        }
    }
    for q in pts.iter_mut().filter(|q| q.0 >= start && q.0 <= end) {
        q.1 = (q.1 + delta).clamp(-144.0, 36.0);
    }
    pts.sort_by_key(|q| q.0);
    c.gain_db = 0.0;
    c.gain_env = pts;
    Ok(json!({}))
}

fn gain_point_remove(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.gain_point_remove";
    let cid = clip_id(p, id)?;
    let at = p.get("at").and_then(Value::as_i64).ok_or_else(|| bad(id, "`at` required"))?;
    let c = e.session_mut().find_clip_mut(cid).ok_or_else(|| bad(id, "no such clip"))?;
    let pts: Vec<(Samples, f32)> = c.gain_env.iter().filter(|q| q.0 != at).map(|&(o, _)| (o, clip_db(c, o))).collect();
    if pts.len() < 2 {
        let g = pts.first().map_or(c.gain_db, |q| q.1);
        c.gain_env.clear();
        c.gain_db = g;
    } else {
        set_clip_env(c, pts);
    }
    Ok(json!({}))
}

fn gain(e: &mut Engine, p: &Value) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let abs = p.get("db").and_then(Value::as_f64);
    let delta = p.get("delta_db").and_then(Value::as_f64);
    if abs.is_none() && delta.is_none() {
        return Err(bad("clip.gain", "`db` or `delta_db` required"));
    }
    let s = e.session_mut();
    for id in &ids {
        if let Some(c) = s.find_clip_mut(*id) {
            let v = abs.unwrap_or(f64::from(c.gain_db) + delta.unwrap_or(0.0));
            c.gain_db = if v.is_finite() { (v as f32).clamp(-144.0, 36.0) } else { c.gain_db };
        }
    }
    Ok(json!({"clips": ids.len()}))
}

fn nudge_gain(e: &mut Engine, p: &Value, db: f64) -> Result<Value> {
    let clips = clip_ids_param(e, p);
    gain(e, &json!({"clips": clips, "delta_db": db}))
}

fn quantize_to_grid(e: &mut Engine, p: &Value) -> Result<Value> {
    let ids = clip_ids_param(e, p);
    let s = e.session();
    let moves: Vec<(openpt_model::ClipId, i64)> =
        ids.iter().filter_map(|id| s.find_clip(*id).map(|(_, c)| (*id, crate::edit::grid_delta(s, c, 0, false)))).collect();
    let s = e.session_mut();
    for (id, d) in &moves {
        crate::edit::move_clips(s, &[*id], *d, &[]);
    }
    Ok(json!({"clips": moves.len()}))
}

#[cfg(test)]
mod tests {
    use openpt_model::{ClipId, TrackId};
    use serde_json::json;

    #[test]
    fn mono_pairs_join_on_stereo_tracks_and_stereo_files_split_over_mono_tracks() {
        use openpt_audio_io::{AudioBuffer, FileFormat};
        let mut e = crate::Engine::default();
        let rate = e.session().sample_rate.hz();
        let s = e.session_mut();
        let l = crate::io::add_source(s, "Gtr.L", AudioBuffer { sample_rate: rate, channels: vec![vec![0.5; 100]] }, None, FileFormat::Wav);
        let r = crate::io::add_source(s, "Gtr.R", AudioBuffer { sample_rate: rate, channels: vec![vec![-0.5; 80]] }, None, FileFormat::Wav);
        let st = e.execute("track.new", &json!({"format": "stereo"})).unwrap()["tracks"][0].as_u64().unwrap();
        e.execute("clip.place_sources", &json!({"sources": [l.0, r.0], "track": st, "at": 0})).unwrap();
        let clip = e.session().track(TrackId(st)).unwrap().clips()[0].clone();
        assert_eq!(clip.name, "Gtr");
        let openpt_model::ClipContent::Audio { source, .. } = clip.content else { panic!("audio clip") };
        let joined = e.session().pool.get(source).unwrap().clone();
        assert_eq!(joined.buffer.num_channels(), 2);
        assert_eq!(joined.frames(), 100, "the shorter file is padded");
        assert_eq!(joined.buffer.channels[1][0], -0.5);
        assert_eq!(joined.buffer.channels[1][90], 0.0);
        let monos = e.execute("track.new", &json!({"format": "mono", "count": 2})).unwrap()["tracks"].clone();
        let (a, b) = (monos[0].as_u64().unwrap(), monos[1].as_u64().unwrap());
        e.execute("clip.place_sources", &json!({"sources": [source.0], "track": a, "at": 0})).unwrap();
        let names = |e: &crate::Engine, t: u64| e.session().track(TrackId(t)).unwrap().clips().iter().map(|c| c.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&e, a), ["Gtr.L"]);
        assert_eq!(names(&e, b), ["Gtr.R"]);
        assert!(e.execute("clip.place_sources", &json!({"sources": [source.0], "track": b, "at": 0})).is_err(), "one mono track is not enough");
        e.execute("edit.undo", &json!({})).unwrap();
        assert!(names(&e, a).is_empty() && names(&e, b).is_empty(), "one undo step");
    }

    #[test]
    fn elastic_warp_renders_and_remove_warp_restores() {
        let mut e = crate::demo::demo_engine();
        let pad = e.session().track_by_name("Pad").map(|t| t.id.0).unwrap();
        e.execute("track.elastic", &json!({"track": pad, "algorithm": "polyphonic"})).unwrap();
        let cid = e.session().track(TrackId(pad)).unwrap().clips()[0].id.0;
        let len0 = e.session().find_clip(ClipId(cid)).unwrap().1.length;
        let r = e.execute("clip.elastic_properties", &json!({"clips": [cid], "ratio": 1.5})).unwrap();
        assert_eq!(r["rendered"], 1);
        let c = e.session().find_clip(ClipId(cid)).unwrap().1.clone();
        assert!((c.length as f64 - len0 as f64 * 1.5).abs() < 2.0, "{} vs {}", c.length, len0);
        assert_eq!(c.stretch, 1.0);
        e.execute("clip.elastic_properties", &json!({"clips": [cid], "ratio": 0.5})).unwrap();
        let c = e.session().find_clip(ClipId(cid)).unwrap().1.clone();
        assert!((c.length as f64 - len0 as f64 * 0.5).abs() < 2.0);
        e.execute("clip.remove_warp", &json!({"clips": [cid]})).unwrap();
        assert_eq!(e.session().find_clip(ClipId(cid)).unwrap().1.length, len0);
    }

    fn elastic_clip(source_len: usize, tone_at: usize, clip_len: i64) -> (crate::Engine, TrackId, ClipId) {
        use openpt_model::{ChannelFormat, Clip, Session};
        let mut s = Session::default();
        let t = s.add_track(openpt_model::TrackKind::Audio, ChannelFormat::Mono, Some("Elastic"));
        let samples = (0..source_len).map(|i| if i >= tone_at { (i as f32 * 0.2).sin() * 0.4 } else { 0.0 }).collect();
        let buf = openpt_audio_io::AudioBuffer { sample_rate: 48_000, channels: vec![samples] };
        let src = crate::io::add_source(&mut s, "Tone", buf, None, openpt_audio_io::FileFormat::Wav);
        let cid = s.new_clip_id();
        crate::edit::place_clip(&mut s, t, Clip::audio(cid, "Clip", src, 0, 0, clip_len));
        let mut e = crate::Engine::new(s);
        e.execute("track.elastic", &json!({"track": t.0, "algorithm": "polyphonic"})).unwrap();
        (e, t, cid)
    }

    fn rendered_peak(e: &crate::Engine, t: TrackId, cid: ClipId) -> f32 {
        let range = e.session().find_clip(cid).unwrap().1.range();
        openpt_mix::render_clips(e.session(), t, range).iter().flatten().fold(0.0, |m, x| m.max(x.abs()))
    }

    #[test]
    fn elastic_properties_stretches_the_clip_material() {
        let (mut e, t, cid) = elastic_clip(20_000, 10_000, 20_000);
        e.execute("clip.elastic_properties", &json!({"clips": [cid.0], "ratio": 2.0})).unwrap();
        assert_eq!(e.session().find_clip(cid).unwrap().1.length, 40_000);
        assert!(rendered_peak(&e, t, cid) > 0.3, "the tone in the second half was lost");

        let (mut e, t, cid) = elastic_clip(60_000, 20_000, 20_000);
        e.execute("clip.elastic_properties", &json!({"clips": [cid.0], "ratio": 0.5})).unwrap();
        assert_eq!(rendered_peak(&e, t, cid), 0.0, "audio from past the clip end leaked in");

        for (ratio, len) in [(4.0, 80_000), (0.2, 4_000)] {
            let (mut e, _, cid) = elastic_clip(20_000, 10_000, 20_000);
            e.execute("clip.elastic_properties", &json!({"clips": [cid.0], "ratio": ratio})).unwrap();
            assert_eq!(e.session().find_clip(cid).unwrap().1.length, len, "ratio {ratio}");
        }
    }

    #[test]
    fn remove_warp_keeps_the_audio_when_the_original_file_is_gone() {
        let mut e = crate::demo::demo_engine();
        let pad = e.session().track_by_name("Pad").map(|t| t.id).unwrap();
        e.execute("track.elastic", &json!({"track": pad, "algorithm": "polyphonic"})).unwrap();
        let clip = e.session().track(pad).unwrap().clips()[0].clone();
        e.execute("clip.elastic_properties", &json!({"clips": [clip.id], "ratio": 1.5})).unwrap();
        e.execute("clip.clear", &json!({"sources": [clip.source()]})).unwrap();
        let r = e.execute("clip.remove_warp", &json!({"clips": [clip.id]})).unwrap();
        assert_eq!(r["restored"], 0);
        let src = e.session().find_clip(clip.id).and_then(|(_, c)| c.source()).unwrap();
        assert!(e.session().source(src).is_some());
    }

    #[test]
    fn gain_nudges_one_db_at_a_time_up_to_the_ceiling() {
        let mut e = crate::demo::demo_engine();
        let kick = e.session().track_by_name("Kick").unwrap().clips()[0].id;
        let gain = |e: &crate::Engine| e.session().find_clip(kick).unwrap().1.gain_db;
        e.execute("edit.select", &json!({"clips": [kick.0]})).unwrap();
        e.execute("clip.gain_nudge_up", &json!({})).unwrap();
        e.execute("clip.gain_nudge_up", &json!({})).unwrap();
        e.execute("clip.gain_nudge_down", &json!({})).unwrap();
        assert_eq!(gain(&e), 1.0);
        e.execute("clip.gain", &json!({"db": 35.5})).unwrap();
        e.execute("clip.gain_nudge_up", &json!({})).unwrap();
        assert_eq!(gain(&e), 36.0);
    }
}
