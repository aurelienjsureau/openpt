use super::more_util::{set_text, store_value, text, toggle_flag, values_with_prefix};
use super::*;
use crate::cmd;
use openpt_model::{BusId, ChannelFormat, ClipContent, OutputPath, Route, Session, SourceId, TrackId, TrackKind};
use openpt_time::Timecode;
use serde_json::json;
use std::path::{Path, PathBuf};

macro_rules! option {
    ($id:literal, $label:literal, $flag:literal) => {
        cmd!(noundo $id, $label, ["Options"], None, "{value?: bool} (toggles when omitted)", always, |e, p| toggle_flag(e, p, $flag))
    };
}

const MIDI_FILTER_KINDS: [&str; 8] = ["notes", "pitch_bend", "mono_aftertouch", "poly_aftertouch", "program", "cc", "sysex", "realtime"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        option!("options.destructive_punch", "DestructivePunch", "options.destructive_punch"),
        cmd!(noundo "options.transport_online", "Transport Online", ["Options"], Some("Cmd+J"), "{value?: bool} (toggles when omitted)", always, |e, p| toggle_flag(e, p, "options.transport_online")),
        option!("options.dynamic_transport", "Dynamic Transport", "options.dynamic_transport"),
        option!(
            "options.marker_target_follows_selection",
            "Marker Target Follows First Selected Track",
            "options.marker_target_follows_first_selected_track"
        ),
        option!("options.midi_thru", "MIDI Thru", "options.midi_thru"),
        option!("options.auto_spot_clips", "Auto-Spot Clips", "options.auto_spot_clips"),
        option!("options.keyboard_lock", "Edit/Tool Mode Keyboard Lock", "options.edit_tool_mode_keyboard_lock"),
        option!("options.calibration_mode", "Calibration Mode", "options.calibration_mode"),
        option!("options.midi_live_mode", "MIDI Live Mode", "options.midi_live_mode"),
        option!("options.low_latency_monitoring", "Low Latency Monitoring", "options.low_latency_monitoring"),
        cmd!(
            "options.prepare_dpe_tracks",
            "Prepare DPE Tracks",
            ["Options"],
            None,
            "{tracks?} — consolidates each record-armed (or given) audio track into one continuous clip for destructive punch",
            always,
            prepare_dpe
        ),
        cmd!(noundo "setup.hardware", "Hardware...", ["Setup"], None, "{device?, buffer_size?: 16..8192} — audio interface settings (sample rate is the session's)", always, |e, p| engine_settings(e, p)),
        cmd!(noundo "setup.playback_engine", "Playback Engine...", ["Setup"], None, "{device?, buffer_size?: 16..8192, delay_compensation?: bool}", always, |e, p| {
            if let Some(v) = p.get("delay_compensation").and_then(Value::as_bool) {
                e.session_mut().edit.delay_compensation = v;
            }
            engine_settings(e, p)
        }),
        cmd!(noundo "setup.disk_allocation", "Disk Allocation...", ["Setup"], None, "{path?, track?} — the record folder for the session, or for one track", always, disk_allocation),
        cmd!(noundo "setup.peripherals", "Peripherals...", ["Setup"], None, "{settings?: {name: number|bool|string}} — control surface / sync device settings", always, peripherals),
        cmd!(
            "setup.io",
            "I/O...",
            ["Setup"],
            None,
            "{action?: list|create_bus|create_busses|set_bus|rename_bus|delete_bus|create_output|rename_output|delete_output|set_output|create_input|rename_input|delete_input|set_input, name?, new_name?, format?: stereo, first_channel?: 0, count?: 1 (create_busses: `name` is the prefix)}",
            always,
            io_setup
        ),
        cmd!(
            "setup.current_timecode",
            "Current Timecode Position...",
            ["Setup"],
            None,
            "{timecode: 'HH:MM:SS:FF'} — sets the session start so the insertion point reads this timecode",
            always,
            current_timecode
        ),
        cmd!(noundo "setup.current_feet_frames", "Current Feet+Frames Position...", ["Setup"], None, "{feet_frames: 'F+FF'} — the feet+frames reading at the insertion point", always, current_feet_frames),
        cmd!(noundo "setup.external_timecode_offset", "External Timecode Offset...", ["Setup"], None, "{frames?: ±, samples?: ±} — offset applied to incoming timecode", always, |e, p| offset_setting(e, p, "sync.external_offset")),
        cmd!(noundo "setup.video_sync_offset", "Video Sync Offset...", ["Setup"], None, "{frames?: ±, samples?: ±} — offset between audio and video playback", always, |e, p| offset_setting(e, p, "video.sync_offset")),
        cmd!(noundo "setup.click_countoff", "Click/Countoff...", ["Setup"], None, "{volume_db?: -60..12, accent_note?: 0..127, normal_note?: 0..127, accent_velocity?, normal_velocity?, countoff?: bool, countoff_bars?: 1..16, only_during_record?: bool}", always, click_countoff),
        cmd!(query "setup.keyboard_shortcuts", "Keyboard Shortcuts...", ["Setup"], None, "{filter?} — every command shortcut", always, |_, p| {
            let f = str_param(p, "filter").map(str::to_ascii_lowercase);
            let rows: Vec<Value> = command_specs()
                .iter()
                .filter_map(|c| c.shortcut.map(|sc| (c, sc)))
                .filter(|(c, _)| f.as_ref().is_none_or(|f| c.id.contains(f.as_str()) || c.label.to_ascii_lowercase().contains(f.as_str())))
                .map(|(c, sc)| json!({"id": c.id, "label": c.label, "menu": c.menu.join(" > "), "shortcut": sc}))
                .collect();
            Ok(json!({"shortcuts": rows}))
        }),
        cmd!(noundo "setup.midi_studio", "MIDI Studio...", ["Setup", "MIDI"], None, "{devices?: [names]} — the MIDI devices in the studio", always, |e, p| {
            let s = e.session_mut();
            if let Some(a) = p.get("devices").and_then(Value::as_array) {
                let names: Vec<&str> = a.iter().filter_map(Value::as_str).take(64).collect();
                set_text(&mut s.edit, "midi.studio.devices", Some(&names.join("|")));
            }
            let devices: Vec<String> = text(&s.edit, "midi.studio.devices").map(|t| t.split('|').map(str::to_string).collect()).unwrap_or_default();
            Ok(json!({"devices": devices}))
        }),
        cmd!(noundo "setup.midi_beat_clock", "MIDI Beat Clock...", ["Setup", "MIDI"], None, "{enabled?: bool, offset?: samples, destination?: name}", always, |e, p| {
            let s = e.session_mut();
            if let Some(v) = p.get("enabled").and_then(Value::as_bool) {
                s.edit.set_flag("midi.beat_clock", v);
            }
            store_value(&mut s.edit, p, "offset", "midi.beat_clock.offset", -1e7, 1e7);
            if let Some(d) = str_param(p, "destination") {
                set_text(&mut s.edit, "midi.beat_clock.destination", Some(d));
            }
            Ok(json!({"enabled": s.edit.flag("midi.beat_clock"), "offset": s.edit.value("midi.beat_clock.offset", 0.0), "destination": text(&s.edit, "midi.beat_clock.destination")}))
        }),
        cmd!(noundo "setup.midi_input_filter", "MIDI Input Filter...", ["Setup", "MIDI"], None, "{filter?: [notes|pitch_bend|mono_aftertouch|poly_aftertouch|program|cc|sysex|realtime], allow?: [...]} — filtered kinds are not recorded", always, midi_filter),
        cmd!(noundo "setup.midi_input_devices", "MIDI Input Devices...", ["Setup", "MIDI"], None, "{enable?: [names], disable?: [names]}", always, midi_inputs),
        cmd!(noundo "setup.transcription_settings", "Transcription Settings...", ["Setup"], None, "{enabled?: bool, language?: 'en'}", always, |e, p| {
            let s = e.session_mut();
            if let Some(v) = p.get("enabled").and_then(Value::as_bool) {
                s.edit.set_flag("transcription.enabled", v);
            }
            if let Some(l) = str_param(p, "language") {
                set_text(&mut s.edit, "transcription.language", Some(l));
            }
            Ok(json!({"enabled": s.edit.flag("transcription.enabled"), "language": text(&s.edit, "transcription.language").unwrap_or_else(|| "en".into())}))
        }),
        cmd!(
            "file.import_session_data",
            "Session Data...",
            ["File", "Import"],
            Some("Alt+Shift+I"),
            "{path, tracks?: [names], dest?: {name: existing track}, data?: {clips, alternate_playlists, inserts, sends, io, volume, pan, mute_solo, automation, comments: bool}, markers?, tempo?, link?} — imports tracks from another session",
            always,
            import_session_data
        ),
        cmd!(query "file.session_data_tracks", "Session Data Tracks", [], None, "{path} — another session's tracks, for Import Session Data", always, session_data_tracks),
        cmd!(query "file.linked_sessions", "Linked Sessions", [], None, "{} — linked sessions saved since their last import", always, linked_sessions),
        cmd!(
            "file.update_linked_session",
            "Update Linked Session",
            [],
            None,
            "{path, dismiss?} — re-imports a linked session's tracks into the tracks of the same name",
            always,
            update_linked_session
        ),
    ]
}

fn prepare_dpe(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "options.prepare_dpe_tracks";
    let s = e.session();
    let tracks: Vec<TrackId> = if p.get("tracks").is_some() || p.get("track").is_some() {
        tracks_param(e, id, p)?
    } else {
        s.tracks.iter().filter(|t| t.mixer.record_arm).map(|t| t.id).collect()
    };
    let tracks: Vec<TrackId> = tracks.into_iter().filter(|t| s.track(*t).is_some_and(|x| x.kind == TrackKind::Audio)).collect();
    if tracks.is_empty() {
        return Err(bad(id, "record-enable (or pass) the audio tracks to prepare"));
    }
    let end = s.content_end();
    if end <= 0 {
        return Err(bad(id, "there is no audio to prepare"));
    }
    let r = openpt_time::Range::new(0, end);
    let mut n = 0;
    for t in tracks {
        let tr = crate::io::bounded_to_track(e.session(), t, r);
        if crate::io::consolidate(e, id, t, tr)? {
            n += 1;
        }
    }
    e.session_mut().edit.set_flag("options.dpe_prepared", true);
    Ok(json!({"prepared": n}))
}

fn engine_settings(e: &mut Engine, p: &Value) -> Result<Value> {
    let s = e.session_mut();
    if let Some(b) = p.get("buffer_size").and_then(Value::as_u64) {
        let b = b.clamp(16, 8192).next_power_of_two().min(8192);
        s.edit.values.insert("engine.buffer_size".into(), b as f64);
    }
    if let Some(d) = str_param(p, "device") {
        set_text(&mut s.edit, "engine.device", Some(d));
    }
    Ok(json!({
        "device": text(&s.edit, "engine.device"),
        "buffer_size": s.edit.value("engine.buffer_size", 1024.0),
        "sample_rate": s.sample_rate.hz(),
        "bit_depth": s.bit_depth,
        "delay_compensation": s.edit.delay_compensation,
    }))
}

fn disk_allocation(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "setup.disk_allocation";
    let t = track_param(e, id, p, "track")?;
    let s = e.session_mut();
    if let Some(path) = str_param(p, "path") {
        let key = t.map_or_else(|| "disk.root".to_string(), |t| format!("disk.track.{}", t.0));
        set_text(&mut s.edit, &key, Some(path));
    }
    let root = text(&s.edit, "disk.root").unwrap_or_else(|| "Audio Files".into());
    let rows: Vec<Value> = s
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .map(|t| json!({"track": t.id, "name": t.name, "path": text(&s.edit, &format!("disk.track.{}", t.id.0)).unwrap_or_else(|| root.clone())}))
        .collect();
    Ok(json!({"root": root, "tracks": rows}))
}

fn peripherals(e: &mut Engine, p: &Value) -> Result<Value> {
    let s = e.session_mut();
    if let Some(m) = p.get("settings").and_then(Value::as_object) {
        for (k, v) in m.iter().take(256) {
            let key: String = k.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')).take(64).collect();
            if key.is_empty() {
                continue;
            }
            let key = format!("peripherals.{key}");
            match v {
                Value::Bool(b) => s.edit.set_flag(&key, *b),
                Value::Number(n) => {
                    if let Some(x) = n.as_f64().filter(|x| x.is_finite()) {
                        s.edit.values.insert(key, x);
                    }
                }
                Value::String(t) => set_text(&mut s.edit, &key, Some(t)),
                _ => {}
            }
        }
    }
    let flags: Vec<&String> = s.edit.flags.iter().filter(|f| f.starts_with("peripherals.")).collect();
    Ok(json!({"values": values_with_prefix(&s.edit, "peripherals."), "flags": flags}))
}

fn io_state(s: &Session) -> Value {
    json!({"busses": s.busses, "outputs": s.outputs, "inputs": s.inputs})
}

fn io_setup(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "setup.io";
    let action = str_param(p, "action").unwrap_or("list");
    if action == "list" {
        return Ok(io_state(e.session()));
    }
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(id, "`name` required"))?.to_string();
    let new_name = str_param(p, "new_name").map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    let format = match str_param(p, "format") {
        Some(f) => ChannelFormat::from_id(f).ok_or_else(|| bad(id, format!("unknown format `{f}`")))?,
        None => ChannelFormat::Stereo,
    };
    let s = e.session_mut();
    match action {
        "create_bus" => {
            if s.bus_by_name(&name).is_some() {
                return Err(bad(id, format!("a bus named `{name}` exists")));
            }
            s.add_bus(&name, format);
        }
        "set_bus" => {
            let b = s.busses.iter_mut().find(|b| b.name.eq_ignore_ascii_case(&name)).ok_or_else(|| bad(id, format!("no bus `{name}`")))?;
            b.format = format;
        }
        "create_busses" => {
            let count = i64_or(p, "count", 1).clamp(1, 64);
            let mut made = Vec::new();
            let mut k = 1;
            while (made.len() as i64) < count && k < 10_000 {
                let n = format!("{name} {k}");
                if s.bus_by_name(&n).is_none() {
                    s.add_bus(&n, format);
                    made.push(n);
                }
                k += 1;
            }
            return Ok(json!({"created": made}));
        }
        "rename_bus" => {
            let to = new_name.ok_or_else(|| bad(id, "`new_name` required"))?;
            if s.bus_by_name(&to).is_some() {
                return Err(bad(id, format!("a bus named `{to}` exists")));
            }
            let b = s.busses.iter_mut().find(|b| b.name.eq_ignore_ascii_case(&name)).ok_or_else(|| bad(id, format!("no bus `{name}`")))?;
            b.name = to;
        }
        "delete_bus" => {
            let bid: BusId = s.bus_by_name(&name).map(|b| b.id).ok_or_else(|| bad(id, format!("no bus `{name}`")))?;
            s.busses.retain(|b| b.id != bid);
            for t in &mut s.tracks {
                if t.mixer.input == Route::Bus(bid) {
                    t.mixer.input = Route::None;
                }
                if t.mixer.output == Route::Bus(bid) {
                    t.mixer.output = Route::None;
                }
                for slot in &mut t.mixer.sends {
                    if slot.as_ref().is_some_and(|x| x.target == Route::Bus(bid)) {
                        *slot = None;
                    }
                }
            }
        }
        "create_output" => {
            if s.outputs.iter().any(|o| o.name.eq_ignore_ascii_case(&name)) {
                return Err(bad(id, format!("an output named `{name}` exists")));
            }
            let first = i64_or(p, "first_channel", 0).clamp(0, 1023) as u16;
            s.outputs.push(OutputPath { name, first_channel: first, format });
        }
        "rename_output" => {
            let to = new_name.ok_or_else(|| bad(id, "`new_name` required"))?;
            let o = s.outputs.iter_mut().find(|o| o.name.eq_ignore_ascii_case(&name)).ok_or_else(|| bad(id, format!("no output `{name}`")))?;
            let old = std::mem::replace(&mut o.name, to.clone());
            for t in &mut s.tracks {
                if t.mixer.output == Route::Hardware(old.clone()) {
                    t.mixer.output = Route::Hardware(to.clone());
                }
            }
        }
        "delete_output" => {
            let before = s.outputs.len();
            s.outputs.retain(|o| !o.name.eq_ignore_ascii_case(&name));
            if s.outputs.len() == before {
                return Err(bad(id, format!("no output `{name}`")));
            }
        }
        "create_input" => {
            if s.inputs.iter().any(|o| o.name.eq_ignore_ascii_case(&name)) {
                return Err(bad(id, format!("an input named `{name}` exists")));
            }
            let first = i64_or(p, "first_channel", 0).clamp(0, 1023) as u16;
            s.inputs.push(openpt_model::InputPath { name, first_channel: first, format });
        }
        "rename_input" => {
            let to = new_name.ok_or_else(|| bad(id, "`new_name` required"))?;
            if s.inputs.iter().any(|o| o.name.eq_ignore_ascii_case(&to)) {
                return Err(bad(id, format!("an input named `{to}` exists")));
            }
            let i = s.inputs.iter_mut().find(|o| o.name.eq_ignore_ascii_case(&name)).ok_or_else(|| bad(id, format!("no input `{name}`")))?;
            let old = std::mem::replace(&mut i.name, to.clone());
            for t in &mut s.tracks {
                if t.mixer.input == Route::Hardware(old.clone()) {
                    t.mixer.input = Route::Hardware(to.clone());
                }
            }
        }
        "delete_input" => {
            let found = s
                .inputs
                .iter()
                .find(|o| o.name.eq_ignore_ascii_case(&name))
                .map(|o| o.name.clone())
                .ok_or_else(|| bad(id, format!("no input `{name}`")))?;
            s.inputs.retain(|o| o.name != found);
            for t in &mut s.tracks {
                if t.mixer.input == Route::Hardware(found.clone()) {
                    t.mixer.input = Route::None;
                }
            }
        }
        "set_input" | "set_output" => {
            let paths = if action == "set_input" { &mut s.inputs } else { &mut s.outputs };
            let path = paths.iter_mut().find(|o| o.name.eq_ignore_ascii_case(&name)).ok_or_else(|| bad(id, format!("no path `{name}`")))?;
            if p.get("first_channel").is_some() {
                path.first_channel = i64_or(p, "first_channel", 0).clamp(0, 1023) as u16;
            }
            if p.get("format").is_some() {
                path.format = format;
            }
        }
        other => return Err(bad(id, format!("unknown action `{other}`"))),
    }
    Ok(io_state(s))
}

fn current_timecode(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "setup.current_timecode";
    let tc = str_param(p, "timecode").ok_or_else(|| bad(id, "`timecode` required (HH:MM:SS:FF)"))?;
    let tc = Timecode::parse(tc).map_err(|err| bad(id, err.to_string()))?;
    let s = e.session_mut();
    let at = s.edit.selection.start.max(0);
    let abs = tc.to_samples(s.frame_rate, s.sample_rate);
    s.timecode_start = abs - at;
    Ok(json!({"timecode_start": s.timecode_start, "at": at}))
}

fn current_feet_frames(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "setup.current_feet_frames";
    let text_v = str_param(p, "feet_frames").ok_or_else(|| bad(id, "`feet_frames` required (F+FF)"))?;
    let (ft, fr) = text_v.split_once('+').ok_or_else(|| bad(id, "use F+FF, e.g. 12+08"))?;
    let ft: i64 = ft.trim().parse().map_err(|_| bad(id, "bad feet"))?;
    let fr: i64 = fr.trim().parse().map_err(|_| bad(id, "bad frames"))?;
    if !(0..=10_000_000).contains(&ft) || !(0..16).contains(&fr) {
        return Err(bad(id, "feet must be 0.. and frames 0..15"));
    }
    let s = e.session_mut();
    let at = s.edit.selection.start.max(0);
    let frames_at = s.frame_rate.frames_at(at, s.sample_rate);
    let start = ft * 16 + fr - frames_at;
    s.edit.values.insert(openpt_model::FEET_FRAMES_START.into(), start as f64);
    Ok(json!({"feet_frames_start": start}))
}

fn offset_setting(e: &mut Engine, p: &Value, key: &str) -> Result<Value> {
    let s = e.session_mut();
    let sr = s.sample_rate;
    let per_frame = s.frame_rate.samples_per_frame(sr);
    if let Some(f) = p.get("frames").and_then(Value::as_f64).filter(|v| v.is_finite()) {
        s.edit.values.insert(key.to_string(), (f.clamp(-10_000.0, 10_000.0) * per_frame).round());
    } else if let Some(n) = p.get("samples").and_then(Value::as_f64).filter(|v| v.is_finite()) {
        let lim = sr.as_f64() * 3600.0;
        s.edit.values.insert(key.to_string(), n.clamp(-lim, lim).round());
    }
    let samples = s.edit.value(key, 0.0);
    Ok(json!({"samples": samples, "frames": samples / per_frame.max(1e-9)}))
}

fn click_countoff(e: &mut Engine, p: &Value) -> Result<Value> {
    let s = e.session_mut();
    store_value(&mut s.edit, p, "volume_db", "click.volume_db", -60.0, 12.0);
    store_value(&mut s.edit, p, "accent_note", "click.accent_note", 0.0, 127.0);
    store_value(&mut s.edit, p, "normal_note", "click.normal_note", 0.0, 127.0);
    store_value(&mut s.edit, p, "accent_velocity", "click.accent_velocity", 1.0, 127.0);
    store_value(&mut s.edit, p, "normal_velocity", "click.normal_velocity", 1.0, 127.0);
    if let Some(v) = p.get("countoff").and_then(Value::as_bool) {
        s.edit.countoff = v;
    }
    if let Some(b) = p.get("countoff_bars").and_then(Value::as_u64) {
        s.edit.countoff_bars = b.clamp(1, 16) as u32;
    }
    if let Some(v) = p.get("only_during_record").and_then(Value::as_bool) {
        s.edit.set_flag("click.only_during_record", v);
    }
    Ok(json!({
        "volume_db": s.edit.value("click.volume_db", -6.0),
        "accent_note": s.edit.value("click.accent_note", 84.0),
        "normal_note": s.edit.value("click.normal_note", 72.0),
        "accent_velocity": s.edit.value("click.accent_velocity", 127.0),
        "normal_velocity": s.edit.value("click.normal_velocity", 100.0),
        "countoff": s.edit.countoff,
        "countoff_bars": s.edit.countoff_bars,
        "only_during_record": s.edit.flag("click.only_during_record"),
    }))
}

fn kinds(p: &Value, key: &str) -> Vec<&'static str> {
    p.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).filter_map(|k| MIDI_FILTER_KINDS.iter().copied().find(|x| *x == k)).collect())
        .unwrap_or_default()
}

fn midi_filter(e: &mut Engine, p: &Value) -> Result<Value> {
    let s = e.session_mut();
    for k in kinds(p, "filter") {
        s.edit.set_flag(&format!("midi.filter.{k}"), true);
    }
    for k in kinds(p, "allow") {
        s.edit.set_flag(&format!("midi.filter.{k}"), false);
    }
    let filtered: Vec<&str> = MIDI_FILTER_KINDS.iter().copied().filter(|k| s.edit.flag(&format!("midi.filter.{k}"))).collect();
    Ok(json!({"filtered": filtered}))
}

fn device_key(name: &str) -> String {
    let n: String = name.chars().filter(|c| !c.is_control() && *c != '=').take(64).collect();
    format!("midi.input.{}", n.trim())
}

fn midi_inputs(e: &mut Engine, p: &Value) -> Result<Value> {
    let s = e.session_mut();
    for (key, on) in [("enable", true), ("disable", false)] {
        for n in p.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).take(64).collect::<Vec<_>>()).unwrap_or_default() {
            if !n.trim().is_empty() {
                s.edit.set_flag(&device_key(n), on);
            }
        }
    }
    let enabled: Vec<&str> = s.edit.flags.iter().filter_map(|f| f.strip_prefix("midi.input.")).collect();
    Ok(json!({"enabled": enabled}))
}

pub const TRACK_DATA: [&str; 10] = ["clips", "alternate_playlists", "inserts", "sends", "io", "volume", "pan", "mute_solo", "automation", "comments"];

fn donor_session(path: &str) -> Result<Session> {
    if crate::io::is_ptx_path(path) {
        crate::io::ptx_session(path)
    } else {
        let text_v = crate::io::read_text(path)?;
        Session::from_json(&text_v).map_err(|err| EngineError::Io(format!("{path}: {err}")))
    }
}

fn modified_secs(path: &str) -> u64 {
    crate::io::modified_secs(path)
}

fn session_data_tracks(_e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "file.session_data_tracks";
    let path = str_param(p, "path").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(id, "`path` required"))?;
    let other = donor_session(path)?;
    let folder = |t: &openpt_model::Track| t.folder.and_then(|f| other.track(f)).map(|f| f.name.clone());
    let tracks: Vec<Value> = other
        .tracks
        .iter()
        .filter(|t| !matches!(t.kind, TrackKind::Master | TrackKind::Video))
        .map(|t| {
            json!({
                "name": t.name,
                "kind": t.kind.id(),
                "clips": t.playlists.get(t.active_playlist).map_or(0, |pl| pl.clips.len()),
                "playlists": t.playlists.len(),
                "folder": folder(t),
            })
        })
        .collect();
    Ok(json!({"tracks": tracks, "markers": other.markers.len(), "tempo_changes": other.tempo.tempos().len(), "name": other.name}))
}

fn import_session_data(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "file.import_session_data";
    let path = str_param(p, "path").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(id, "`path` required"))?.to_string();
    let other = donor_session(&path)?;
    let dir = Path::new(&path).parent().map(Path::to_path_buf).unwrap_or_default();
    let names: Option<Vec<String>> =
        p.get("tracks").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
    let picked: Vec<openpt_model::Track> = match &names {
        Some(ns) => {
            let mut v = Vec::new();
            for n in ns {
                v.push(other.track_by_name(n).cloned().ok_or_else(|| bad(id, format!("`{path}` has no track named `{n}`")))?);
            }
            v
        }
        None => other.tracks.iter().filter(|t| t.kind != TrackKind::Master).cloned().collect(),
    };
    let wants = |k: &str| p.get("data").and_then(|d| d.get(k)).and_then(Value::as_bool).unwrap_or(true);
    let (markers, tempo) = (bool_or(p, "markers", false), bool_or(p, "tempo", false));
    if picked.is_empty() && !markers && !tempo {
        return Err(bad(id, "nothing to import"));
    }
    let dest_of = |n: &str| p.get("dest").and_then(|d| d.get(n)).and_then(Value::as_str).map(str::to_string);
    let mut needed: Vec<SourceId> = if wants("clips") {
        picked.iter().flat_map(|t| t.playlists.iter().flat_map(|pl| pl.clips.iter().filter_map(|c| c.source()))).collect()
    } else {
        Vec::new()
    };
    if wants("inserts") {
        needed.extend(picked.iter().filter_map(|t| t.instrument.as_ref().and_then(|i| i.sample)));
    }
    needed.sort_by_key(|x| x.0);
    needed.dedup();
    let sr = e.session().sample_rate.hz();
    let s = e.session_mut();
    let mut files = crate::io::DecodedFiles::new();
    let mut map: std::collections::BTreeMap<u64, SourceId> = Default::default();
    let mut missing = Vec::new();
    for src in needed.iter().filter_map(|sid| other.source(*sid)) {
        let abs = if Path::new(&src.path).is_absolute() { PathBuf::from(&src.path) } else { dir.join(&src.path) };
        let mut ns = src.clone();
        ns.id = SourceId(s.alloc());
        ns.path = abs.to_string_lossy().into_owned();
        match crate::io::decode_file(&mut files, &abs, sr) {
            Ok(audio) => {
                ns.frames = audio.buffer.frames() as u64;
                ns.sample_rate = sr;
                ns.unsaved = true;
                s.pool.insert(ns.id, audio);
            }
            Err(_) => {
                ns.unsaved = false;
                missing.push(ns.path.clone());
            }
        }
        map.insert(src.id.0, ns.id);
        s.sources.push(ns);
    }
    let mut out = Vec::new();
    let targets: Vec<Option<TrackId>> = picked.iter().map(|t| dest_of(&t.name).and_then(|n| s.track_by_name(&n).map(|x| x.id))).collect();
    let new_ids: std::collections::BTreeMap<u64, TrackId> =
        picked.iter().zip(&targets).map(|(t, to)| (t.id.0, to.unwrap_or_else(|| TrackId(s.alloc())))).collect();
    for (mut tr, into) in picked.into_iter().zip(targets) {
        tr.id = new_ids[&tr.id.0];
        tr.folder = tr.folder.and_then(|f| new_ids.get(&f.0).copied());
        tr.mixer.vca = tr.mixer.vca.and_then(|v| new_ids.get(&v).map(|n| n.0));
        for pl in &mut tr.playlists {
            for c in &mut pl.clips {
                c.id = s.new_clip_id();
                if let ClipContent::Audio { source, .. } = &mut c.content
                    && let Some(n) = map.get(&source.0)
                {
                    *source = *n;
                }
            }
        }
        if let Some(ins) = tr.instrument.as_mut() {
            ins.sample = ins.sample.and_then(|sid| map.get(&sid.0).copied());
        }
        let remap = |s: &mut Session, r: &Route| -> Route {
            match r {
                Route::Bus(b) => other.bus(*b).map_or(Route::None, |ob| Route::Bus(s.add_bus(&ob.name, ob.format))),
                x => x.clone(),
            }
        };
        tr.mixer.input = remap(s, &tr.mixer.input);
        tr.mixer.output = remap(s, &tr.mixer.output);
        for slot in tr.mixer.sends.iter_mut().flatten() {
            slot.target = remap(s, &slot.target);
        }
        if !wants("alternate_playlists") && tr.playlists.len() > 1 {
            let main = tr.playlists.swap_remove(tr.active_playlist.min(tr.playlists.len() - 1));
            tr.playlists = vec![main];
            tr.active_playlist = 0;
        }
        let mut t = match into.and_then(|t| s.track(t)) {
            Some(t) => t.clone(),
            None => {
                let mut b = openpt_model::Track::new(tr.id, s.unique_track_name(&tr.name), tr.kind, tr.format, tr.color);
                b.folder = tr.folder;
                b.mixer.vca = tr.mixer.vca;
                b.height = tr.height;
                b.view.clone_from(&tr.view);
                b.elastic.clone_from(&tr.elastic);
                b.ticks_timebase = tr.ticks_timebase;
                b.midi_channel = tr.midi_channel;
                if tr.is_folder() {
                    b.mixer.input = tr.mixer.input.clone();
                }
                b
            }
        };
        if tr.folder.is_some() {
            t.folder = tr.folder;
        }
        if tr.mixer.vca.is_some() {
            t.mixer.vca = tr.mixer.vca;
        }
        if wants("clips") {
            let imported = std::mem::take(&mut tr.playlists);
            let old = std::mem::replace(&mut t.playlists, imported);
            t.active_playlist = tr.active_playlist.min(t.playlists.len().saturating_sub(1));
            if into.is_some() {
                for mut pl in old.into_iter().filter(|pl| !pl.clips.is_empty()) {
                    pl.name = format!("{}.old", pl.name);
                    t.playlists.push(pl);
                }
            }
        }
        if wants("inserts") {
            t.mixer.inserts = tr.mixer.inserts;
            t.instrument = tr.instrument;
        }
        if wants("sends") {
            t.mixer.sends = tr.mixer.sends;
        }
        if wants("io") {
            t.mixer.input = tr.mixer.input;
            t.mixer.output = tr.mixer.output;
        }
        if wants("volume") {
            t.mixer.volume_db = tr.mixer.volume_db;
            t.mixer.trim_db = tr.mixer.trim_db;
        }
        if wants("pan") && t.mixer.pan.len() == tr.mixer.pan.len() {
            t.mixer.pan = tr.mixer.pan;
            t.mixer.surround = tr.mixer.surround;
        }
        if wants("mute_solo") {
            t.mixer.mute = tr.mixer.mute;
            t.mixer.solo = tr.mixer.solo;
        }
        if wants("automation") {
            t.automation = tr.automation;
            t.mixer.automation_mode = tr.mixer.automation_mode;
        }
        if wants("comments") {
            t.comments = tr.comments;
        }
        out.push(t.id);
        match into.and_then(|x| s.track_index(x)) {
            Some(i) => s.tracks[i] = t,
            None => {
                let pos = s.tracks.iter().position(|x| x.kind == TrackKind::Master).unwrap_or(s.tracks.len());
                s.tracks.insert(pos, t);
            }
        }
    }
    for f in out.iter().copied().filter(|f| s.track(*f).is_some_and(openpt_model::Track::is_folder)).collect::<Vec<_>>() {
        let members: Vec<TrackId> = out.iter().copied().filter(|m| s.track(*m).is_some_and(|t| t.folder == Some(f))).collect();
        super::track::place_under(s, f, &members);
    }
    if markers {
        for m in &other.markers {
            if !s.markers.iter().any(|x| x.name == m.name && x.start == m.start) {
                let id = s.add_marker(&m.name, m.kind, m.start, m.end);
                if let Some(x) = s.markers.iter_mut().find(|x| x.id == id) {
                    x.comments.clone_from(&m.comments);
                    x.color = m.color;
                }
            }
        }
    }
    if tempo {
        s.tempo = other.tempo.clone();
    }
    if bool_or(p, "link", false) {
        let tracks: Vec<String> =
            names.unwrap_or_else(|| other.tracks.iter().filter(|t| t.kind != TrackKind::Master).map(|t| t.name.clone()).collect());
        let data: serde_json::Map<String, Value> = TRACK_DATA.iter().map(|k| ((*k).to_string(), json!(wants(k)))).collect();
        s.linked_sessions.retain(|l| l.path != path);
        s.linked_sessions.push(openpt_model::SessionLink { path: path.clone(), tracks, data: Value::Object(data), modified: modified_secs(&path) });
    }
    if !out.is_empty() {
        s.edit.selected_tracks = out.clone();
    }
    Ok(json!({"tracks": out, "sources": map.len(), "missing": missing}))
}

fn linked_sessions(e: &mut Engine, _p: &Value) -> Result<Value> {
    let changed: Vec<Value> = e
        .session()
        .linked_sessions
        .iter()
        .filter(|l| modified_secs(&l.path) > l.modified)
        .map(|l| json!({"path": l.path, "tracks": l.tracks}))
        .collect();
    Ok(json!({"changed": changed}))
}

fn update_linked_session(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "file.update_linked_session";
    let path = str_param(p, "path").ok_or_else(|| bad(id, "`path` required"))?.to_string();
    let link = e.session().linked_sessions.iter().find(|l| l.path == path).cloned().ok_or_else(|| bad(id, format!("`{path}` is not linked")))?;
    let mut r = json!({"tracks": []});
    if !bool_or(p, "dismiss", false) {
        let dest: serde_json::Map<String, Value> =
            link.tracks.iter().filter(|n| e.session().track_by_name(n).is_some()).map(|n| (n.clone(), json!(n))).collect();
        r = import_session_data(e, &json!({"path": path, "tracks": link.tracks, "data": link.data, "dest": dest}))?;
    }
    if let Some(l) = e.session_mut().linked_sessions.iter_mut().find(|l| l.path == path) {
        l.modified = modified_secs(&path);
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_flags_toggle() {
        let mut e = Engine::default();
        assert_eq!(e.execute("options.midi_thru", &json!({})).unwrap()["value"], true);
        assert!(e.session().edit.flag("options.midi_thru"));
        e.execute("options.midi_thru", &json!({"value": false})).unwrap();
        assert!(!e.session().edit.flag("options.midi_thru"));
    }

    #[test]
    fn busses_are_created_several_at_once_in_any_format() {
        let mut e = Engine::default();
        let r = e.execute("setup.io", &json!({"action": "create_busses", "name": "FX", "count": 3, "format": "mono"})).unwrap();
        assert_eq!(r["created"], json!(["FX 1", "FX 2", "FX 3"]));
        assert_eq!(e.session().bus_by_name("FX 2").map(|b| b.format), Some(ChannelFormat::Mono));
        e.execute("setup.io", &json!({"action": "set_bus", "name": "FX 2", "format": "5.1"})).unwrap();
        assert_eq!(e.session().bus_by_name("FX 2").map(|b| b.format.channels()), Some(6));
        let r = e.execute("setup.io", &json!({"action": "create_busses", "name": "FX", "count": 1})).unwrap();
        assert_eq!(r["created"], json!(["FX 4"]));
    }
    #[test]
    fn input_paths_route_recording_and_follow_renames() {
        let mut e = Engine::default();
        assert_eq!(e.session().input_first_channel("In 3"), 2);
        assert_eq!(e.session().input_first_channel("In 5-6"), 4);
        e.execute("setup.io", &json!({"action": "create_input", "name": "Guitar DI", "first_channel": 9, "format": "mono"})).unwrap();
        assert_eq!(e.session().input_first_channel("Guitar DI"), 9);
        let t = e.execute("track.new", &json!({"format": "mono"})).unwrap()["tracks"][0].as_u64().unwrap();
        e.execute("track.input", &json!({"track": t, "input": "Guitar DI"})).unwrap();
        e.execute("setup.io", &json!({"action": "set_input", "name": "Guitar DI", "first_channel": 3})).unwrap();
        assert_eq!(e.session().input_first_channel("Guitar DI"), 3);
        e.execute("setup.io", &json!({"action": "rename_input", "name": "Guitar DI", "new_name": "Gtr"})).unwrap();
        let input = |e: &Engine| e.session().track(openpt_model::TrackId(t)).map(|x| x.mixer.input.clone());
        assert_eq!(input(&e), Some(Route::Hardware("Gtr".into())));
        assert!(e.execute("setup.io", &json!({"action": "create_input", "name": "gtr"})).is_err(), "names are unique");
        e.execute("setup.io", &json!({"action": "delete_input", "name": "Gtr"})).unwrap();
        assert_eq!(input(&e), Some(Route::None));
        let listed = e.execute("setup.io", &json!({})).unwrap();
        assert_eq!(listed["inputs"].as_array().map(Vec::len), Some(12));
    }

    #[test]
    fn io_bus_lifecycle() {
        let mut e = crate::demo::demo_engine();
        e.execute("setup.io", &json!({"action": "create_bus", "name": "Drums"})).unwrap();
        e.execute("setup.io", &json!({"action": "rename_bus", "name": "Verb", "new_name": "Hall"})).unwrap();
        assert!(e.session().bus_by_name("Hall").is_some());
        e.execute("setup.io", &json!({"action": "delete_bus", "name": "Hall"})).unwrap();
        assert!(
            e.session().tracks.iter().all(|t| t
                .mixer
                .sends
                .iter()
                .flatten()
                .all(|s| matches!(s.target, Route::None | Route::Main | Route::Hardware(_))))
        );
        assert!(e.session().tracks.iter().all(|t| {
            !matches!(t.mixer.input, Route::Bus(_))
                || e.session()
                    .bus(match t.mixer.input {
                        Route::Bus(b) => b,
                        _ => BusId(0),
                    })
                    .is_some()
        }));
        e.execute("setup.io", &json!({"action": "create_output", "name": "Out 3-4", "first_channel": 2})).unwrap();
        assert_eq!(e.session().outputs.len(), 2);
        assert!(e.execute("setup.io", &json!({"action": "nope", "name": "x"})).is_err());
        assert!(e.can_undo());
    }

    #[test]
    fn current_timecode_sets_start() {
        let mut e = Engine::default();
        e.execute("edit.select", &json!({"start": 48_000})).unwrap();
        e.execute("setup.current_timecode", &json!({"timecode": "01:00:00:00"})).unwrap();
        let s = e.session();
        let shown = s.format_position(48_000, openpt_time::TimeFormat::Timecode);
        assert_eq!(shown, "01:00:00:00");
    }

    #[test]
    fn click_countoff_settings() {
        let mut e = Engine::default();
        let r = e.execute("setup.click_countoff", &json!({"volume_db": -12.0, "countoff_bars": 4, "countoff": true})).unwrap();
        assert_eq!(r["volume_db"], -12.0);
        assert_eq!(e.session().edit.countoff_bars, 4);
        assert!(e.session().edit.countoff);
    }

    #[test]
    fn shortcuts_listed() {
        let mut e = Engine::default();
        let r = e.execute("setup.keyboard_shortcuts", &json!({})).unwrap();
        assert!(r["shortcuts"].as_array().unwrap().iter().any(|x| x["id"] == "edit.undo"));
    }

    #[test]
    fn engine_and_midi_settings() {
        let mut e = Engine::default();
        let r = e.execute("setup.playback_engine", &json!({"buffer_size": 300, "device": "Interface"})).unwrap();
        assert_eq!(r["buffer_size"], 512.0);
        assert_eq!(r["device"], "Interface");
        let r = e.execute("setup.midi_input_filter", &json!({"filter": ["sysex", "bogus"]})).unwrap();
        assert_eq!(r["filtered"], json!(["sysex"]));
        let r = e.execute("setup.midi_input_devices", &json!({"enable": ["Keys"]})).unwrap();
        assert_eq!(r["enabled"], json!(["Keys"]));
        let r = e.execute("setup.video_sync_offset", &json!({"frames": 2})).unwrap();
        assert_eq!(r["samples"], 3200.0);
    }

    #[test]
    fn import_session_data_brings_tracks_and_audio() {
        let dir = std::env::temp_dir().join(format!("sc-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("donor.openpt").to_string_lossy().into_owned();
        let mut donor = crate::demo::demo_engine();
        crate::io::save_session(&mut donor, &path).unwrap();
        let mut e = Engine::default();
        let r = e.execute("file.import_session_data", &json!({"path": path, "tracks": ["Kick", "Keys"]})).unwrap();
        assert_eq!(r["tracks"].as_array().unwrap().len(), 2);
        assert_eq!(r["missing"], json!([]));
        let kick = e.session().track_by_name("Kick").unwrap();
        let src = kick.clips()[0].source().unwrap();
        assert!(e.session().pool.get(src).is_some());
        assert_eq!(e.session().track_by_name("Keys").unwrap().clips().len(), 1);
        assert!(e.execute("file.import_session_data", &json!({"path": path, "tracks": ["Nope"]})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_session_data_picks_its_data_keeps_folders_and_follows_a_link() {
        let dir = std::env::temp_dir().join(format!("sc-import-pick-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("donor.openpt").to_string_lossy().into_owned();
        let mut donor = crate::demo::demo_engine();
        donor.execute("track.move_to_new_folder", &json!({"tracks": ["Kick", "Snare"], "name": "Drums"})).unwrap();
        donor.execute("mix.volume", &json!({"track": "Kick", "db": -6.0})).unwrap();
        crate::io::save_session(&mut donor, &path).unwrap();
        let listed = Engine::default().execute("file.session_data_tracks", &json!({"path": path})).unwrap();
        assert!(listed["tracks"].as_array().unwrap().iter().any(|t| t["name"] == "Kick" && t["folder"] == "Drums"));

        let mut e = crate::demo::demo_engine();
        let clips_before = e.session().track_by_name("Kick").unwrap().clips().len();
        let r = e
            .execute(
                "file.import_session_data",
                &json!({"path": path, "tracks": ["Drums", "Kick", "Snare"], "dest": {"Kick": "Kick"}, "data": {"clips": false}, "link": true}),
            )
            .unwrap();
        assert_eq!(r["tracks"].as_array().unwrap().len(), 3);
        let s = e.session();
        let kick = s.track_by_name("Kick").unwrap();
        assert_eq!(kick.clips().len(), clips_before, "clips stay when not imported");
        assert!((kick.mixer.volume_db + 6.0).abs() < 1e-4);
        let folder = s.tracks.iter().find(|t| t.is_folder()).unwrap();
        assert_eq!(kick.folder, Some(folder.id));
        assert_eq!(s.linked_sessions.len(), 1);
        assert_eq!(e.execute("file.linked_sessions", &json!({})).unwrap()["changed"], json!([]));
        e.session_mut().linked_sessions[0].modified = 0;
        assert_eq!(e.execute("file.linked_sessions", &json!({})).unwrap()["changed"].as_array().unwrap().len(), 1);
        e.execute("file.update_linked_session", &json!({"path": path})).unwrap();
        assert_eq!(e.execute("file.linked_sessions", &json!({})).unwrap()["changed"], json!([]));
        assert_eq!(e.session().tracks.iter().filter(|t| t.name.starts_with("Kick")).count(), 1, "re-imported into the same track");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_session_data_brings_a_samplers_sample() {
        let dir = std::env::temp_dir().join(format!("sc-import-sampler-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("hit.wav");
        let buf = openpt_audio_io::AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.5; 4_800]] };
        std::fs::write(&wav, openpt_audio_io::encode(&buf, &openpt_audio_io::EncodeOptions::default()).unwrap()).unwrap();
        let mut donor = Engine::default();
        donor.execute("track.new", &json!({"count": 1, "name": "Pads", "kind": "instrument"})).unwrap();
        donor.execute("mix.sampler_load", &json!({"track": "Pads", "path": wav.to_string_lossy()})).unwrap();
        let path = dir.join("donor.openpt").to_string_lossy().into_owned();
        crate::io::save_session(&mut donor, &path).unwrap();
        let mut e = crate::demo::demo_engine();
        e.execute("file.import_session_data", &json!({"path": path, "tracks": ["Pads"]})).unwrap();
        let sample = e.session().track_by_name("Pads").and_then(|t| t.instrument.as_ref()).and_then(|i| i.sample).unwrap();
        let src = e.session().source(sample).unwrap();
        assert_eq!(src.name, "hit");
        assert!(e.session().pool.get(sample).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
