use super::*;
use openpt_audio_io::{AudioBuffer, FileFormat};
use openpt_model::{Clip, ClipContent, EditState, Session, SourceId, TrackId};
use openpt_time::Samples;
use serde_json::json;

pub fn toggle_flag(e: &mut Engine, p: &Value, id: &str) -> Result<Value> {
    let s = e.session_mut();
    let on = p.get("value").and_then(Value::as_bool).unwrap_or(!s.edit.flag(id));
    s.edit.set_flag(id, on);
    Ok(json!({"flag": id, "value": on}))
}

pub fn exclusive_flag(e: &mut Engine, id: &str, group: &[&str]) -> Result<Value> {
    let s = e.session_mut();
    for g in group {
        s.edit.set_flag(g, false);
    }
    s.edit.set_flag(id, true);
    Ok(json!({"flag": id, "value": true}))
}

pub fn set_flags(e: &mut Engine, ids: &[&str], on: bool) -> Value {
    let s = e.session_mut();
    for id in ids {
        s.edit.set_flag(id, on);
    }
    json!({"flags": ids, "value": on})
}

pub fn set_text(edit: &mut EditState, key: &str, value: Option<&str>) {
    let prefix = format!("{key}=");
    edit.flags.retain(|f| !f.starts_with(&prefix));
    if let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) {
        let v: String = v.chars().take(200).collect();
        edit.flags.insert(format!("{prefix}{v}"));
    }
}

pub fn text(edit: &EditState, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    edit.flags.iter().find_map(|f| f.strip_prefix(&prefix).map(str::to_string))
}

pub fn values_with_prefix(edit: &EditState, prefix: &str) -> Value {
    let m: serde_json::Map<String, Value> =
        edit.values.iter().filter_map(|(k, v)| k.strip_prefix(prefix).map(|rest| (rest.to_string(), json!(v)))).collect();
    Value::Object(m)
}

pub fn store_value(edit: &mut EditState, p: &Value, key: &str, target: &str, lo: f64, hi: f64) -> bool {
    match p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()) {
        Some(v) => {
            edit.values.insert(target.to_string(), v.clamp(lo, hi));
            true
        }
        None => false,
    }
}

pub fn track_key(prefix: &str, t: TrackId, name: &str) -> String {
    format!("{prefix}.{}.{name}", t.0)
}

pub fn target_playlist(s: &Session, t: TrackId) -> Option<usize> {
    let v = s.edit.values.get(&format!("playlist.target.{}", t.0)).copied()?;
    if !v.is_finite() || v < 0.0 {
        return None;
    }
    let i = v as usize;
    s.track(t).filter(|tr| i < tr.playlists.len()).map(|_| i)
}

pub fn activate_playlist(s: &mut Session, t: TrackId, idx: usize) -> bool {
    let Some(tr) = s.track_mut(t) else { return false };
    if idx >= tr.playlists.len() || idx == tr.active_playlist {
        return false;
    }
    let prev = tr.active_playlist;
    tr.active_playlist = idx;
    s.edit.values.insert(format!("playlist.recent.{}", t.0), prev as f64);
    true
}

pub fn env_db(env: &[(Samples, f32)], at: Samples) -> f32 {
    let mut prev: Option<(Samples, f32)> = None;
    for &(t, v) in env {
        if t >= at {
            return match prev {
                Some((pt, pv)) if t > pt => pv + (v - pv) * ((at - pt) as f32 / (t - pt) as f32),
                _ => v,
            };
        }
        prev = Some((t, v));
    }
    prev.map_or(0.0, |(_, v)| v)
}

pub fn clip_db(c: &Clip, rel: Samples) -> f32 {
    c.gain_db + if c.gain_env.is_empty() { 0.0 } else { env_db(&c.gain_env, rel) }
}

pub fn set_clip_env(c: &mut Clip, mut pts: Vec<(Samples, f32)>) {
    pts.retain(|(o, v)| *o >= 0 && *o <= c.length && v.is_finite());
    pts.sort_by_key(|p| p.0);
    pts.dedup_by_key(|p| p.0);
    for p in &mut pts {
        p.1 = p.1.clamp(-144.0, 36.0);
    }
    c.gain_db = 0.0;
    let flat = pts.windows(2).all(|w| w.first().map(|a| a.1) == w.get(1).map(|b| b.1));
    if flat {
        c.gain_db = pts.first().map_or(0.0, |p| p.1);
        c.gain_env.clear();
    } else {
        c.gain_env = pts;
    }
}

fn max_render(s: &Session) -> Samples {
    s.sample_rate.samples(3600.0).min(openpt_dsp::offline::MAX_OFFLINE_SAMPLES as Samples)
}

pub fn render_stretched(s: &mut Session, cmd: &str, c: &Clip, out_len: Samples) -> Result<Option<SourceId>> {
    let ClipContent::Audio { source, offset } = c.content else { return Ok(None) };
    let Some(audio) = s.pool.get(source).cloned() else { return Ok(None) };
    let max = max_render(s);
    if out_len <= 0 || out_len > max {
        let secs = max as f64 / s.sample_rate.as_f64();
        return Err(bad(cmd, format!("the target length must be between 1 sample and {secs:.0} s")));
    }
    let stretch = if c.stretch.is_finite() && c.stretch > 0.0 { c.stretch } else { 1.0 };
    let src_len = ((c.length as f64) / stretch).round().max(1.0);
    let ratio = out_len as f64 / src_len;
    if !(0.1..=10.0).contains(&ratio) {
        return Err(bad(cmd, format!("stretch ratio {ratio:.3} is outside 0.1..10")));
    }
    let a = usize::try_from(offset.max(0)).unwrap_or(0);
    let n = src_len as usize;
    let ch: Vec<Vec<f32>> = audio
        .buffer
        .channels
        .iter()
        .map(|x| {
            let mut v: Vec<f32> = x.iter().skip(a).take(n).copied().collect();
            v.resize(n, 0.0);
            v
        })
        .collect();
    let sr = s.sample_rate;
    let mut out = openpt_dsp::offline::time_stretch(&ch, ratio, sr.as_f64() as f32);
    let want = usize::try_from(out_len).unwrap_or(0);
    for o in &mut out {
        o.resize(want, 0.0);
    }
    let buf = AudioBuffer { sample_rate: sr.hz(), channels: out };
    Ok(Some(crate::io::add_source(s, &format!("{}-TCE", c.name), buf, None, FileFormat::Wav)))
}

pub fn replace_clip(s: &mut Session, t: TrackId, old: openpt_model::ClipId, new: Clip) {
    if let Some(pl) = s.track_mut(t).and_then(|tr| tr.playlist_mut()) {
        pl.clips.retain(|c| c.id != old);
    }
    crate::edit::place_clip(s, t, new);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_settings_round_trip() {
        let mut ed = EditState::default();
        set_text(&mut ed, "engine.device", Some("Built-in"));
        set_text(&mut ed, "engine.device", Some("Interface"));
        assert_eq!(text(&ed, "engine.device").as_deref(), Some("Interface"));
        set_text(&mut ed, "engine.device", None);
        assert_eq!(text(&ed, "engine.device"), None);
    }

    #[test]
    fn envelope_interpolates() {
        let env = [(0, 0.0f32), (100, -10.0)];
        assert!((env_db(&env, 50) + 5.0).abs() < 1e-6);
        assert_eq!(env_db(&[], 50), 0.0);
        assert_eq!(env_db(&env, 500), -10.0);
    }

    #[test]
    fn stretches_are_no_longer_than_time_stretch_makes() {
        let mut s = Session::default();
        assert_eq!(max_render(&s), s.sample_rate.samples(3600.0));
        s.sample_rate = openpt_time::SampleRate::new(96_000).unwrap();
        assert_eq!(max_render(&s), openpt_dsp::offline::MAX_OFFLINE_SAMPLES as Samples);
    }
}
