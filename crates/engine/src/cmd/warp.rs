use super::*;
use crate::cmd;
use openpt_model::{Clip, ClipContent, ClipId, Session, SourceId, Warp, WarpMarker};
use serde_json::json;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("clip.warp_add", "Add Warp Marker", [], None, "{clip: id, at: position}", always, add),
        cmd!("clip.warp_move", "Move Warp Marker", [], None, "{clip: id, index: n, to: position}", always, move_marker),
        cmd!("clip.warp_remove", "Remove Warp Marker", [], None, "{clip: id, index: n}", always, remove),
        cmd!(
            query "clip.warp_events",
            "Warp Events",
            [],
            None,
            "{clip: id, sensitivity?: 0..1} → {events: [position], markers: [{index, at, src}]} (session positions)",
            always,
            events
        ),
    ]
}

fn clip_param(e: &Engine, id: &str, p: &Value) -> Result<(TrackId, Clip)> {
    let c = p.get("clip").and_then(Value::as_u64).map(ClipId).ok_or_else(|| bad(id, "`clip` id required"))?;
    e.session().find_clip(c).map(|(t, c)| (t, c.clone())).ok_or_else(|| bad(id, "no such clip"))
}

fn algorithm(s: &Session, id: &str, t: TrackId) -> Result<String> {
    s.track(t)
        .and_then(|x| x.elastic.clone())
        .filter(|a| a != "none")
        .ok_or_else(|| bad(id, "Elastic Audio is off on this track: choose an algorithm first"))
}

fn current(c: &Clip, w: &Warp) -> bool {
    let ClipContent::Audio { source, offset } = c.content else { return false };
    match w.rendered {
        Some(r) => source == r && offset == 0,
        None => source == w.source && offset == w.offset && c.length == w.length,
    }
}

pub(crate) fn warp_of(id: &str, c: &Clip) -> Result<Warp> {
    if let Some(w) = c.warp.as_ref().filter(|w| current(c, w)) {
        return Ok(w.clone());
    }
    let ClipContent::Audio { source, offset } = c.content else { return Err(bad(id, "only audio clips warp")) };
    Ok(Warp { source, offset, length: c.length.max(1), markers: Vec::new(), rendered: None })
}

pub(crate) fn render(s: &mut Session, id: &str, cid: ClipId, mut w: Warp, clip_len: Samples, algo: &str) -> Result<()> {
    let audio = s.pool.get(w.source).cloned().ok_or_else(|| bad(id, "the clip's audio is not loaded"))?;
    let max = super::more_util::max_render_samples(s);
    if clip_len <= 0 || clip_len > max {
        return Err(bad(id, format!("a warped clip must be between 1 sample and {:.0} s", max as f64 / s.sample_rate.as_f64())));
    }
    let sr = s.sample_rate;
    let nch = audio.buffer.num_channels().max(1);
    let mut out: Vec<Vec<f32>> = vec![Vec::with_capacity(usize::try_from(clip_len).unwrap_or(0)); nch];
    for pair in w.anchors(clip_len).windows(2) {
        let [a, b] = pair else { continue };
        let n = usize::try_from((b.src - a.src).max(1)).unwrap_or(1);
        let want = usize::try_from((b.at - a.at).max(1)).unwrap_or(1);
        let from = usize::try_from((w.offset + a.src).max(0)).unwrap_or(0);
        let seg: Vec<Vec<f32>> = audio
            .buffer
            .channels
            .iter()
            .map(|c| {
                let mut v: Vec<f32> = c.iter().skip(from).take(n).copied().collect();
                v.resize(n, 0.0);
                v
            })
            .collect();
        let done = if n == want {
            seg
        } else if algo == "varispeed" {
            openpt_dsp::offline::resample(&seg, u32::try_from(n).unwrap_or(u32::MAX), u32::try_from(want).unwrap_or(u32::MAX))
        } else {
            openpt_dsp::offline::time_stretch(&seg, want as f64 / n as f64, sr.as_f64() as f32)
        };
        for (o, mut d) in out.iter_mut().zip(done) {
            d.resize(want, 0.0);
            o.extend(d);
        }
    }
    let name = s.find_clip(cid).map(|(_, c)| c.name.clone()).unwrap_or_default();
    let buf = openpt_audio_io::AudioBuffer { sample_rate: sr.hz(), channels: out };
    let format = s.source(w.source).map_or(openpt_audio_io::FileFormat::Wav, |x| x.format);
    let new = crate::io::add_source(s, &format!("{name} warped"), buf, None, format);
    let old = w.rendered.replace(new);
    if let Some(c) = s.find_clip_mut(cid) {
        c.content = ClipContent::Audio { source: new, offset: 0 };
        c.length = clip_len;
        c.stretch = 1.0;
        c.warp = Some(w);
    }
    if let Some(old) = old {
        drop_if_unused(s, old);
    }
    Ok(())
}

pub(crate) fn drop_if_unused(s: &mut Session, src: SourceId) {
    let used = s
        .tracks
        .iter()
        .flat_map(|t| t.playlists.iter())
        .flat_map(|p| &p.clips)
        .any(|c| matches!(c.content, ClipContent::Audio { source, .. } if source == src));
    if !used {
        s.sources.retain(|x| x.id != src);
        s.pool.remove(src);
    }
}

fn add(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.warp_add";
    let (t, c) = clip_param(e, id, p)?;
    algorithm(e.session(), id, t)?;
    let at = position_param(e, id, p, "at")?.ok_or_else(|| bad(id, "`at` required"))? - c.start;
    if at <= 0 || at >= c.length {
        return Err(bad(id, "a warp marker goes inside the clip"));
    }
    let mut w = warp_of(id, &c)?;
    let src = w.to_src(c.length, at);
    if w.markers.iter().any(|m| (m.src - src).abs() < 2) {
        return Ok(json!({"index": w.markers.iter().position(|m| (m.src - src).abs() < 2)}));
    }
    w.markers.push(WarpMarker { src, at });
    w.markers.sort_by_key(|m| m.src);
    let index = w.markers.iter().position(|m| m.src == src);
    if let Some(cm) = e.session_mut().find_clip_mut(c.id) {
        cm.warp = Some(w);
    }
    if !bool_or(p, "exact", false) {
        for pc in partners(e.session(), t, c.start + at) {
            let _ = add(e, &json!({"clip": pc.0, "at": c.start + at, "exact": true}));
        }
    }
    Ok(json!({"index": index}))
}

fn move_marker(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.warp_move";
    let (t, c) = clip_param(e, id, p)?;
    let algo = algorithm(e.session(), id, t)?;
    let to = position_param(e, id, p, "to")?.ok_or_else(|| bad(id, "`to` required"))? - c.start;
    let mut w = warp_of(id, &c)?;
    let index = p.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(id, "`index` required"))?;
    let lo = if index == 0 { 0 } else { w.markers.get(index - 1).map_or(0, |m| m.at) } + 1;
    let hi = w.markers.get(index + 1).map_or(c.length, |m| m.at) - 1;
    let m = w.markers.get_mut(index).ok_or_else(|| bad(id, "no such warp marker"))?;
    if hi < lo {
        return Err(bad(id, "no room to move this marker"));
    }
    let old = c.start + m.at;
    m.at = to.clamp(lo, hi);
    let at = m.at;
    render(e.session_mut(), id, c.id, w, c.length, &algo)?;
    if !bool_or(p, "exact", false) {
        for (pc, i) in partner_markers(e.session(), t, old) {
            let _ = move_marker(e, &json!({"clip": pc.0, "index": i, "to": c.start + at, "exact": true}));
        }
    }
    Ok(json!({"at": c.start + at}))
}

fn remove(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.warp_remove";
    let (t, c) = clip_param(e, id, p)?;
    let algo = algorithm(e.session(), id, t)?;
    let mut w = warp_of(id, &c)?;
    let index = p.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(id, "`index` required"))?;
    if index >= w.markers.len() {
        return Err(bad(id, "no such warp marker"));
    }
    let old = c.start + w.markers[index].at;
    w.markers.remove(index);
    render(e.session_mut(), id, c.id, w, c.length, &algo)?;
    if !bool_or(p, "exact", false) {
        for (pc, i) in partner_markers(e.session(), t, old) {
            let _ = remove(e, &json!({"clip": pc.0, "index": i, "exact": true}));
        }
    }
    Ok(json!({}))
}

fn events(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "clip.warp_events";
    let (_, c) = clip_param(e, id, p)?;
    let w = warp_of(id, &c)?;
    let sens = f64_or(p, "sensitivity", 0.5) as f32;
    let s = e.session();
    let audio = s.pool.get(w.source).cloned().ok_or_else(|| bad(id, "the clip's audio is not loaded"))?;
    let from = usize::try_from(w.offset.max(0)).unwrap_or(0);
    let n = usize::try_from(w.length.max(0)).unwrap_or(0);
    let seg: Vec<Vec<f32>> = audio.buffer.channels.iter().map(|ch| ch.iter().skip(from).take(n).copied().collect()).collect();
    let hits = openpt_dsp::offline::detect_transients(&seg, s.sample_rate.as_f64() as f32, sens);
    let events: Vec<Samples> = hits.into_iter().map(|h| c.start + w.to_clip(c.length, h as Samples)).collect();
    let markers: Vec<Value> = w.markers.iter().enumerate().map(|(i, m)| json!({"index": i, "at": c.start + m.at, "src": m.src})).collect();
    Ok(json!({"events": events, "markers": markers}))
}

pub(crate) fn quantize(s: &mut Session, id: &str, cid: ClipId, grid_ticks: i64, strength: f32) -> Result<Vec<(Samples, Samples)>> {
    let (t, c) = s.find_clip(cid).map(|(t, c)| (t, c.clone())).ok_or_else(|| bad(id, "no such clip"))?;
    let algo = algorithm(s, id, t)?;
    let mut w = warp_of(id, &c)?;
    let audio = s.pool.get(w.source).cloned().ok_or_else(|| bad(id, "the clip's audio is not loaded"))?;
    let from = usize::try_from(w.offset.max(0)).unwrap_or(0);
    let n = usize::try_from(w.length.max(0)).unwrap_or(0);
    let seg: Vec<Vec<f32>> = audio.buffer.channels.iter().map(|ch| ch.iter().skip(from).take(n).copied().collect()).collect();
    let sr = s.sample_rate;
    let hits = openpt_dsp::offline::detect_transients(&seg, sr.as_f64() as f32, 0.5);
    let grid = grid_ticks.max(1) as f64;
    let mut pairs = Vec::new();
    let mut markers: Vec<WarpMarker> = Vec::new();
    for h in hits {
        let src = h as Samples;
        let at = w.to_clip(c.length, src);
        let ticks = s.tempo.samples_to_ticks_f(c.start + at, sr);
        let target = s.tempo.tick_to_samples(((ticks / grid).round() * grid) as i64, sr) - c.start;
        let to = at + ((target - at) as f64 * f64::from(strength)).round() as Samples;
        let floor = markers.last().map_or(0, |m| m.at);
        if src <= 0 || src >= w.length || to <= floor || to >= c.length || markers.last().is_some_and(|m| src <= m.src) {
            continue;
        }
        pairs.push((c.start + at, c.start + to));
        markers.push(WarpMarker { src, at: to });
    }
    if markers.is_empty() {
        return Ok(pairs);
    }
    w.markers = markers;
    render(s, id, cid, w, c.length, &algo)?;
    Ok(pairs)
}

pub(crate) fn apply_pairs(s: &mut Session, id: &str, cid: ClipId, pairs: &[(Samples, Samples)]) -> Result<usize> {
    let (t, c) = s.find_clip(cid).map(|(t, c)| (t, c.clone())).ok_or_else(|| bad(id, "no such clip"))?;
    let algo = algorithm(s, id, t)?;
    let mut w = warp_of(id, &c)?;
    let mut markers: Vec<WarpMarker> = Vec::new();
    for &(from, to) in pairs {
        let (f, at) = (from - c.start, to - c.start);
        if f <= 0 || f >= c.length || at <= markers.last().map_or(0, |m| m.at) || at >= c.length {
            continue;
        }
        let src = w.to_src(c.length, f);
        if src <= markers.last().map_or(0, |m| m.src) || src >= w.length {
            continue;
        }
        markers.push(WarpMarker { src, at });
    }
    let n = markers.len();
    if n > 0 {
        w.markers = markers;
        render(s, id, cid, w, c.length, &algo)?;
    }
    Ok(n)
}

fn partner_markers(s: &Session, track: TrackId, at: Samples) -> Vec<(ClipId, usize)> {
    partners(s, track, at)
        .into_iter()
        .filter_map(|pc| {
            let (_, c) = s.find_clip(pc)?;
            let i = c.warp.as_ref()?.markers.iter().position(|m| (c.start + m.at - at).abs() <= 2)?;
            Some((pc, i))
        })
        .collect()
}

pub(crate) fn partners(s: &Session, track: TrackId, at: Samples) -> Vec<ClipId> {
    if s.edit.flag("groups.suspended") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for g in s.groups.iter().filter(|g| g.active && g.edit && g.members.contains(&track)) {
        for m in g.members.iter().filter(|m| **m != track) {
            let Some(tr) = s.track(*m).filter(|tr| tr.elastic.as_deref().is_some_and(|a| a != "none")) else { continue };
            for c in tr.clips().iter().filter(|c| c.is_audio() && at > c.start && at < c.end()) {
                if !out.contains(&c.id) {
                    out.push(c.id);
                }
            }
        }
    }
    out
}

pub(crate) fn stretch(s: &mut Session, id: &str, cid: ClipId, ratio: f64, algo: &str) -> Result<()> {
    let c = s.find_clip(cid).map(|(_, c)| c.clone()).ok_or_else(|| bad(id, "no such clip"))?;
    let mut w = warp_of(id, &c)?;
    let len = openpt_time::to_samples(w.length as f64 * ratio).max(1);
    let k = len as f64 / c.length.max(1) as f64;
    for m in &mut w.markers {
        m.at = openpt_time::to_samples(m.at as f64 * k);
    }
    render(s, id, cid, w, len, algo)
}

pub(crate) fn unwarp(s: &mut Session, cid: ClipId) -> bool {
    let Some(c) = s.find_clip(cid).map(|(_, c)| c.clone()) else { return false };
    let Some(w) = c.warp.clone().filter(|w| current(&c, w)) else { return false };
    if s.source(w.source).is_none() {
        return false;
    }
    if let Some(cm) = s.find_clip_mut(cid) {
        cm.content = ClipContent::Audio { source: w.source, offset: w.offset };
        cm.length = w.length;
        cm.warp = None;
    }
    if let Some(r) = w.rendered {
        drop_if_unused(s, r);
    }
    true
}

#[cfg(test)]
mod tests {
    use crate::Engine;
    use openpt_model::ClipContent;
    use serde_json::json;

    fn warped_clip() -> (Engine, u64, i64) {
        let mut e = Engine::default();
        let rate = e.session().sample_rate.hz();
        let mut ch = vec![0.0f32; rate as usize * 2];
        for k in 0..8 {
            for i in 0..64 {
                ch[k * rate as usize / 4 + i] = 0.9;
            }
        }
        let s = e.session_mut();
        let src = crate::io::add_source(
            s,
            "Clicks",
            openpt_audio_io::AudioBuffer { sample_rate: rate, channels: vec![ch] },
            None,
            openpt_audio_io::FileFormat::Wav,
        );
        let t = e.execute("track.new", &json!({"format": "mono"})).unwrap()["tracks"][0].as_u64().unwrap();
        let clip = e.execute("clip.place_source", &json!({"source": src.0, "track": t, "at": 0})).unwrap()["clip"].as_u64().unwrap();
        (e, clip, t as i64)
    }

    #[test]
    fn quantize_moves_each_event_of_an_elastic_clip_onto_the_grid() {
        let (mut e, clip, t) = warped_clip();
        e.session_mut().find_clip_mut(openpt_model::ClipId(clip)).unwrap().start = 1440;
        assert!(e.execute("event.quantize", &json!({"clips": [clip], "grid": "1/8"})).unwrap()["audio_events"] == 0, "needs Elastic Audio");
        e.execute("track.elastic", &json!({"track": t, "algorithm": "rhythmic"})).unwrap();
        let r = e.execute("event.quantize", &json!({"clips": [clip], "grid": "1/8"})).unwrap();
        assert!(r["audio_events"].as_u64().unwrap() >= 5, "{r}");
        let ev = e.execute("clip.warp_events", &json!({"clip": clip})).unwrap();
        let off: Vec<i64> = ev["events"].as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).filter(|x| *x > 6000).map(|x| (x + 6000) % 12_000 - 6000).collect();
        assert!(off.len() >= 5 && off.iter().all(|d| d.abs() < 300), "events off the 1/8 grid by {off:?}");
    }

    #[test]
    fn grouped_elastic_tracks_warp_together() {
        let (mut e, clip, t) = warped_clip();
        let src = match e.session().find_clip(openpt_model::ClipId(clip)).unwrap().1.content {
            ClipContent::Audio { source, .. } => source.0,
            _ => unreachable!(),
        };
        let t2 = e.execute("track.new", &json!({"format": "mono"})).unwrap()["tracks"][0].as_u64().unwrap();
        let clip2 = e.execute("clip.place_source", &json!({"source": src, "track": t2, "at": 0})).unwrap()["clip"].as_u64().unwrap();
        for tr in [t as u64, t2] {
            e.execute("track.elastic", &json!({"track": tr, "algorithm": "rhythmic"})).unwrap();
        }
        e.execute("track.group", &json!({"name": "Drums", "tracks": [t, t2]})).unwrap();
        let markers = |e: &mut Engine, c: u64| -> Vec<i64> {
            e.execute("clip.warp_events", &json!({"clip": c})).unwrap()["markers"].as_array().unwrap().iter().map(|m| m["at"].as_i64().unwrap()).collect()
        };
        e.execute("clip.warp_add", &json!({"clip": clip, "at": 30_000})).unwrap();
        assert_eq!(markers(&mut e, clip2), vec![30_000], "the add reaches the other member");
        e.execute("clip.warp_move", &json!({"clip": clip, "index": 0, "to": 33_000})).unwrap();
        assert_eq!((markers(&mut e, clip), markers(&mut e, clip2)), (vec![33_000], vec![33_000]));
        e.execute("clip.warp_remove", &json!({"clip": clip, "index": 0})).unwrap();
        assert!(markers(&mut e, clip2).is_empty());
        e.session_mut().find_clip_mut(openpt_model::ClipId(clip2)).unwrap().start = 0;
        e.execute("event.quantize", &json!({"clips": [clip, clip2], "grid": "1/16"})).unwrap();
        assert_eq!(markers(&mut e, clip), markers(&mut e, clip2), "both follow the same events");
    }

    #[test]
    fn markers_need_elastic_audio_then_stretch_the_audio_between_them() {
        let (mut e, clip, t) = warped_clip();
        let rate = i64::from(e.session().sample_rate.hz());
        assert!(e.execute("clip.warp_add", &json!({"clip": clip, "at": rate / 2})).is_err(), "Elastic Audio off");
        e.execute("track.elastic", &json!({"track": t, "algorithm": "polyphonic"})).unwrap();
        let ev = e.execute("clip.warp_events", &json!({"clip": clip})).unwrap();
        let events: Vec<i64> = ev["events"].as_array().unwrap().iter().filter_map(|v| v.as_i64()).collect();
        assert!(events.len() >= 6, "clicks found: {events:?}");
        let i = e.execute("clip.warp_add", &json!({"clip": clip, "at": rate})).unwrap()["index"].as_u64().unwrap();
        e.execute("clip.warp_move", &json!({"clip": clip, "index": i, "to": rate * 3 / 2})).unwrap();
        let c = e.session().find_clip(openpt_model::ClipId(clip)).unwrap().1.clone();
        assert_eq!(c.length, rate * 2);
        let w = c.warp.clone().unwrap();
        assert_eq!(w.to_clip(c.length, rate), rate * 3 / 2);
        let ClipContent::Audio { source, .. } = c.content else { panic!() };
        assert_eq!(e.session().pool.get(source).unwrap().frames() as i64, rate * 2);
        let after: Vec<i64> =
            e.execute("clip.warp_events", &json!({"clip": clip})).unwrap()["events"].as_array().unwrap().iter().filter_map(|v| v.as_i64()).collect();
        assert!(after.iter().any(|x| (x - rate * 3 / 4).abs() < rate / 20), "the click at 0.5 s now plays near 0.75 s: {after:?}");
        let renders = e.session().sources.len();
        e.execute("clip.remove_warp", &json!({"clips": [clip]})).unwrap();
        let c = e.session().find_clip(openpt_model::ClipId(clip)).unwrap().1.clone();
        assert!(c.warp.is_none());
        assert_eq!(e.session().sources.len(), renders - 1);
    }

    #[test]
    fn varispeed_resamples_and_undo_restores() {
        let (mut e, clip, t) = warped_clip();
        let rate = i64::from(e.session().sample_rate.hz());
        e.execute("track.elastic", &json!({"track": t, "algorithm": "varispeed"})).unwrap();
        e.execute("clip.warp_add", &json!({"clip": clip, "at": rate})).unwrap();
        e.execute("clip.warp_move", &json!({"clip": clip, "index": 0, "to": rate / 2})).unwrap();
        assert!(e.session().find_clip(openpt_model::ClipId(clip)).unwrap().1.warp.is_some());
        e.execute("edit.undo", &json!({})).unwrap();
        let c = e.session().find_clip(openpt_model::ClipId(clip)).unwrap().1.clone();
        assert!(c.warp.as_ref().is_none_or(|w| w.rendered.is_none()), "back to before the move");
    }
}
