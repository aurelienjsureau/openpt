use super::*;
use crate::cmd;
use openpt_model::{AutoParam, AutomationMode, INSERT_SLOTS, Insert, SEND_SLOTS, SendSlot, Session, TrackId};
use serde_json::json;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("mix.volume", "Set Volume", [], None, "{tracks?, db: -144..12 | delta_db}", has_selection, volume),
        cmd!("mix.suspend_groups", "Suspend All Groups", [], Some("Cmd+Shift+G"), "{value?}", always, |e, p| {
            let s = e.session_mut();
            let v = p.get("value").and_then(Value::as_bool).unwrap_or(!s.edit.flag("groups.suspended"));
            s.edit.set_flag("groups.suspended", v);
            Ok(json!({"value": v}))
        }),
        cmd!("mix.pan", "Set Pan", [], None, "{tracks?, pan: -1..1 | delta, index?: 0|1}", has_selection, pan),
        cmd!(noundo "mix.mute", "Mute", [], Some("Shift+M"), "{tracks?, value?: bool}", has_selection, |e, p| toggle(
            e,
            p,
            "mix.mute",
            |s, t, v| set_grouped(s, t, "mute", |tr| tr.mixer.mute = v),
            |tr| tr.mixer.mute
        )),
        cmd!(noundo "mix.solo", "Solo", [], Some("Shift+S"), "{tracks?, value?: bool, exclusive?: bool}", has_selection, solo),
        cmd!(noundo "mix.record_safe", "Record Safe", [], None, "{tracks?, value?: bool}", has_selection, |e, p| toggle(
            e,
            p,
            "mix.record_safe",
            |s, t, v| {
                if let Some(tr) = s.track_mut(t) {
                    tr.mixer.record_safe = v;
                    if v {
                        tr.mixer.record_arm = false;
                    }
                }
            },
            |tr| tr.mixer.record_safe
        )),
        cmd!(noundo "mix.solo_safe", "Solo Safe", [], None, "{tracks?, value?: bool}", has_selection, |e, p| toggle(
            e,
            p,
            "mix.solo_safe",
            |s, t, v| {
                if let Some(tr) = s.track_mut(t) {
                    tr.mixer.solo_safe = v
                }
            },
            |tr| tr.mixer.solo_safe
        )),
        cmd!("mix.clear_solos", "Clear All Solos", [], None, "{}", always, |e, _| {
            for t in &mut e.session_mut().tracks {
                t.mixer.solo = false
            }
            Ok(json!({}))
        }),
        cmd!("mix.clear_mutes", "Clear All Mutes", [], None, "{}", always, |e, _| {
            for t in &mut e.session_mut().tracks {
                t.mixer.mute = false
            }
            Ok(json!({}))
        }),
        cmd!(noundo "mix.record_arm", "Record Enable", [], Some("Shift+R"), "{tracks?, value?: bool}", has_selection, |e, p| toggle(
            e,
            p,
            "mix.record_arm",
            |s, t, v| set_grouped(s, t, "record", |tr| if tr.kind.has_playlist() && !(v && tr.mixer.record_safe) {
                tr.mixer.record_arm = v
            }),
            |tr| tr.mixer.record_arm
        )),
        cmd!(noundo "mix.input_monitor", "Input Monitoring", [], Some("Shift+I"), "{tracks?, value?: bool}", has_selection, |e, p| toggle(
            e,
            p,
            "mix.input_monitor",
            |s, t, v| {
                if let Some(tr) = s.track_mut(t) {
                    tr.mixer.input_monitor = v
                }
            },
            |tr| tr.mixer.input_monitor
        )),
        cmd!("mix.phase_invert", "Phase Invert", [], None, "{tracks?, value?: bool}", has_selection, |e, p| toggle(
            e,
            p,
            "mix.phase_invert",
            |s, t, v| {
                if let Some(tr) = s.track_mut(t) {
                    tr.mixer.phase_invert = v
                }
            },
            |tr| tr.mixer.phase_invert
        )),
        cmd!("mix.trim", "Input Trim", [], None, "{tracks?, db}", has_selection, |e, p| {
            let tracks = tracks_required(e, "mix.trim", p)?;
            let db = f32_or(p, "db", 0.0).clamp(-144.0, 24.0);
            let s = e.session_mut();
            for t in tracks {
                if let Some(tr) = s.track_mut(t) {
                    tr.mixer.trim_db = db
                }
            }
            Ok(json!({"db": db}))
        }),
        cmd!(
            "mix.automation_mode",
            "Automation Mode",
            [],
            None,
            "{tracks?, mode: off|read|touch|latch|touch/latch|write|trim}",
            has_selection,
            automation_mode
        ),
        cmd!(
            "mix.insert",
            "Insert Plugin",
            [],
            None,
            "{track?, slot?: 0..9 | a..j, plugin: id, params?: {id: value}, multi_mono?: bool}",
            has_selection,
            insert
        ),
        cmd!("mix.insert_remove", "Remove Insert", [], None, "{track?, slot}", has_selection, insert_remove),
        cmd!("mix.insert_bypass", "Bypass Insert", [], None, "{track?, slot, value?: bool}", has_selection, insert_bypass),
        cmd!(
            "mix.insert_key",
            "Key Input",
            [],
            None,
            "{track?, slot, bus?: name | id | null, on?: bool, listen?: bool} (choosing a bus turns External Key on)",
            has_selection,
            insert_key
        ),
        cmd!("mix.insert_param", "Set Plugin Parameter", [], None, "{track?, slot, param, value}", has_selection, insert_param),
        cmd!(
            "mix.insert_automation",
            "Plug-in Automation",
            [],
            None,
            "{track?, slot, params: [id], enable?: true} — which parameters (master_bypass included) can be automated",
            has_selection,
            insert_automation
        ),
        cmd!(
            "mix.insert_params",
            "Set Plugin Parameters",
            [],
            None,
            "{track?, slot, params: {id: value}, preset?: name} (unknown ids and non-numeric values are errors)",
            has_selection,
            insert_params
        ),
        cmd!(
            "mix.route_to_new_aux",
            "New Aux Track for Bus",
            [],
            None,
            "{tracks?, name?: 'Aux N', format?: stereo, send_slot?: n} — a bus and an Aux track listening to it; the tracks' output (or that send) goes to the bus",
            has_selection,
            route_to_new_aux
        ),
        cmd!("mix.insert_move", "Move Insert", [], None, "{track?, from, to}", has_selection, insert_move),
        cmd!(
            "mix.insert_transfer",
            "Move or Copy Insert",
            [],
            None,
            "{from_track, from, to_track, to, copy?: false} — the insert (plugin, settings, state) into another slot, of any track; it replaces what was there",
            always,
            insert_transfer
        ),
        cmd!(
            "mix.send",
            "Assign Send",
            [],
            None,
            "{track?, slot?: 0..9|a..j, bus: name, level_db?: 0 (kept when re-targeting), pre_fader?: false}",
            has_selection,
            send
        ),
        cmd!("mix.send_remove", "Remove Send", [], None, "{tracks?, slot}", has_selection, |e, p| remove_sends(e, p, "mix.send_remove", true)),
        cmd!("mix.sends_remove_all", "Remove All Sends", ["Track"], None, "{tracks?, slot?} — every send, or `slot`, of each track", has_selection, |e, p| {
            remove_sends(e, p, "mix.sends_remove_all", false)
        }),
        cmd!("mix.send_level", "Send Level", [], None, "{track?, slot, db?, pan?, mute?, pre_fader?}", has_selection, send_level),
        cmd!("mix.new_bus", "New Bus", [], None, "{name, format?: stereo}", always, |e, p| {
            let name = str_param(p, "name").filter(|n| !n.trim().is_empty()).ok_or_else(|| bad("mix.new_bus", "`name` required"))?.to_string();
            let fmt = str_param(p, "format").and_then(openpt_model::ChannelFormat::from_id).unwrap_or(openpt_model::ChannelFormat::Stereo);
            let id = e.session_mut().add_bus(&name, fmt);
            Ok(json!({"bus": id}))
        }),
        cmd!(
            "automation.set_point",
            "Add Automation Breakpoint",
            [],
            None,
            "{track?, param: volume|pan|mute|send_a_level|plugin:a:<id>, at, value, from?}",
            has_selection,
            set_point
        ),
        cmd!("automation.write_range", "Write Automation", [], None, "{track?, param, start?, end?, value}", has_selection, write_range),
        cmd!(
            "automation.move_point",
            "Move Automation Point",
            [],
            None,
            "{track, param, from: position, at: position, value} — the point at `from`, kept between its neighbours",
            always,
            move_point
        ),
        cmd!(
            "automation.trim_range",
            "Trim Automation",
            [],
            None,
            "{track, param, start, end, delta, points_only?} — offsets the segment, anchored just outside (points_only: moves the breakpoints alone)",
            always,
            trim_range
        ),
        cmd!("automation.delete_point", "Delete Automation Point", [], None, "{track, param, at: position}", always, delete_point),
        cmd!("automation.clear", "Clear Automation", [], None, "{track?, param?, start?, end?}", has_selection, clear_lane),
        cmd!("automation.thin", "Thin", ["Edit", "Automation"], Some("Cmd+Alt+T"), "{tracks?, tolerance?: 0.1}", has_selection, |e, p| thin(
            e, p, false
        )),
        cmd!("automation.thin_all", "Thin All", ["Edit", "Automation"], None, "{tolerance?: 0.1}", has_tracks, |e, p| thin(e, p, true)),
        cmd!("automation.write_to_current", "Write to Current", ["Edit", "Automation"], Some("Cmd+/"), "{tracks?}", has_range, |e, p| write_current(
            e, p, false
        )),
        cmd!("automation.write_to_all", "Write to All Enabled", ["Edit", "Automation"], Some("Cmd+Alt+/"), "{tracks?}", has_range, |e, p| {
            write_current(e, p, true)
        }),
        cmd!(
            "automation.volume_to_clip_gain",
            "Convert Volume Automation to Clip Gain",
            ["Edit", "Automation"],
            None,
            "{tracks?}",
            has_range,
            vol_to_clip_gain
        ),
        cmd!(
            "automation.clip_gain_to_volume",
            "Convert Clip Gain to Volume Automation",
            ["Edit", "Automation"],
            None,
            "{tracks?}",
            has_range,
            clip_gain_to_vol
        ),
        cmd!(
            "automation.copy_to_send",
            "Copy to Send...",
            ["Edit", "Automation"],
            Some("Cmd+Alt+H"),
            "{tracks?, send: slot}",
            has_selection,
            copy_to_send
        ),
    ]
}

fn set_grouped(s: &mut Session, t: TrackId, attr: &str, f: impl Fn(&mut openpt_model::Track)) {
    set_grouped_once(s, t, attr, &mut Vec::new(), f);
}

fn set_grouped_once(s: &mut Session, t: TrackId, attr: &str, done: &mut Vec<TrackId>, f: impl Fn(&mut openpt_model::Track)) {
    if done.contains(&t) {
        return;
    }
    let mut targets = vec![t];
    let loose = s.edit.flag("groups.suspended") || s.edit.flag("groups.isolate");
    for g in s.groups.iter().filter(|_| !loose) {
        if g.active && g.mix && g.members.contains(&t) && g.attributes.iter().any(|a| a == attr) {
            for m in &g.members {
                if !targets.contains(m) {
                    targets.push(*m);
                }
            }
        }
    }
    targets.retain(|x| !done.contains(x));
    for id in targets {
        if let Some(tr) = s.track_mut(id) {
            f(tr);
        }
        done.push(id);
    }
}

fn toggle(e: &mut Engine, p: &Value, cmd: &str, set: fn(&mut Session, TrackId, bool), get: fn(&openpt_model::Track) -> bool) -> Result<Value> {
    let tracks = tracks_required(e, cmd, p)?;
    let s = e.session_mut();
    let v = p.get("value").and_then(Value::as_bool).unwrap_or_else(|| !tracks.iter().all(|t| s.track(*t).is_some_and(get)));
    s.edit.set_flag("groups.isolate", bool_or(p, "isolate", false));
    for t in &tracks {
        set(s, *t, v);
    }
    s.edit.set_flag("groups.isolate", false);
    Ok(json!({"value": v}))
}

fn volume(e: &mut Engine, p: &Value) -> Result<Value> {
    let tracks = tracks_required(e, "mix.volume", p)?;
    let abs = p.get("db").and_then(Value::as_f64);
    let delta = p.get("delta_db").and_then(Value::as_f64);
    if abs.is_none() && delta.is_none() {
        return Err(bad("mix.volume", "`db` or `delta_db` required"));
    }
    let s = e.session_mut();
    let isolate = bool_or(p, "isolate", false);
    s.edit.set_flag("groups.isolate", isolate);
    let mut done = Vec::new();
    for t in &tracks {
        let cur = s.track(*t).map_or(0.0, |tr| tr.mixer.volume_db);
        let new = match (abs, delta) {
            (Some(a), _) => a as f32,
            (None, Some(d)) => cur + d as f32,
            _ => cur,
        };
        let new = if new.is_finite() { new.clamp(-144.0, 12.0) } else { cur };
        let d = new - cur;
        set_grouped_once(s, *t, "volume", &mut done, |tr| {
            tr.mixer.volume_db = if tr.id == *t { new } else { (tr.mixer.volume_db + d).clamp(-144.0, 12.0) }
        });
    }
    s.edit.set_flag("groups.isolate", false);
    Ok(json!({"tracks": tracks.len()}))
}

fn pan(e: &mut Engine, p: &Value) -> Result<Value> {
    let tracks = tracks_required(e, "mix.pan", p)?;
    let v = f32_or(p, "pan", 0.0).clamp(-1.0, 1.0);
    let idx = i64_or(p, "index", 0).clamp(0, 1) as usize;
    let delta = p.get("delta").and_then(Value::as_f64).filter(|d| d.is_finite()).map(|d| d as f32);
    let s = e.session_mut();
    for t in &tracks {
        if let Some(tr) = s.track_mut(*t)
            && let Some(slot) = tr.mixer.pan.get_mut(idx)
        {
            *slot = delta.map_or(v, |d| (*slot + d).clamp(-1.0, 1.0));
        }
    }
    Ok(json!({"pan": v}))
}

fn solo(e: &mut Engine, p: &Value) -> Result<Value> {
    let tracks = tracks_required(e, "mix.solo", p)?;
    let exclusive = bool_or(p, "exclusive", e.session().edit.solo_latch() == "xor");
    let s = e.session_mut();
    let v = p.get("value").and_then(Value::as_bool).unwrap_or_else(|| !tracks.iter().all(|t| s.track(*t).is_some_and(|tr| tr.mixer.solo)));
    if exclusive && v {
        for tr in &mut s.tracks {
            tr.mixer.solo = false;
        }
    }
    s.edit.set_flag("groups.isolate", bool_or(p, "isolate", false));
    for t in &tracks {
        set_grouped(s, *t, "solo", |tr| tr.mixer.solo = v);
    }
    s.edit.set_flag("groups.isolate", false);
    Ok(json!({"value": v}))
}

fn automation_mode(e: &mut Engine, p: &Value) -> Result<Value> {
    let tracks = tracks_required(e, "mix.automation_mode", p)?;
    let m = str_param(p, "mode").and_then(AutomationMode::from_id).ok_or_else(|| bad("mix.automation_mode", "unknown mode"))?;
    let s = e.session_mut();
    for t in &tracks {
        if let Some(tr) = s.track_mut(*t) {
            tr.mixer.automation_mode = m;
        }
    }
    Ok(json!({"mode": m}))
}

fn slot_of(p: &Value, key: &str, max: usize) -> Option<usize> {
    let v = p.get(key)?;
    let i = if let Some(n) = v.as_u64() {
        n as usize
    } else {
        let s = v.as_str()?.trim().to_ascii_lowercase();
        let c = s.chars().next()?;
        if s.len() == 1 && c.is_ascii_lowercase() { (c as u8 - b'a') as usize } else { s.parse().ok()? }
    };
    (i < max).then_some(i)
}

fn remove_sends(e: &mut Engine, p: &Value, cmd: &str, slot_required: bool) -> Result<Value> {
    let tracks = tracks_required(e, cmd, p)?;
    let slot = slot_of(p, "slot", SEND_SLOTS);
    if slot.is_none() && (slot_required || p.get("slot").is_some()) {
        return Err(bad(cmd, format!("`slot` must be 0..{} or a..{}", SEND_SLOTS - 1, (b'a' + SEND_SLOTS as u8 - 1) as char)));
    }
    let s = e.session_mut();
    let mut removed = 0;
    for t in tracks {
        if let Some(tr) = s.track_mut(t) {
            for (k, send) in tr.mixer.sends.iter_mut().enumerate() {
                if slot.is_none_or(|n| n == k) && send.take().is_some() {
                    removed += 1
                }
            }
        }
    }
    Ok(json!({"removed": removed}))
}

fn track_slot(e: &Engine, p: &Value, cmd: &str, max: usize) -> Result<(TrackId, usize)> {
    let t = tracks_required(e, cmd, p)?.first().copied().ok_or_else(|| bad(cmd, "no track"))?;
    let slot = slot_of(p, "slot", max).ok_or_else(|| bad(cmd, format!("`slot` must be 0..{} or a..{}", max - 1, (b'a' + max as u8 - 1) as char)))?;
    Ok((t, slot))
}

fn insert(e: &mut Engine, p: &Value) -> Result<Value> {
    let plugin = str_param(p, "plugin").ok_or_else(|| bad("mix.insert", "`plugin` required"))?.to_string();
    let info = super::vst3::plugin_info(&plugin).ok_or_else(|| bad("mix.insert", format!("unknown plugin `{plugin}`")))?;
    let t = tracks_required(e, "mix.insert", p)?.first().copied().ok_or_else(|| bad("mix.insert", "no track"))?;
    let explicit = slot_of(p, "slot", INSERT_SLOTS);
    let s = e.session_mut();
    let tr = s.track_mut(t).ok_or_else(|| bad("mix.insert", "no track"))?;
    let slot =
        explicit.or_else(|| tr.mixer.inserts.iter().position(Option::is_none)).ok_or_else(|| bad("mix.insert", "all insert slots are in use"))?;
    let mut ins = Insert::new(info.id);
    ins.multi_mono = p.get("multi_mono").and_then(Value::as_bool).unwrap_or(false);
    for pi in info.params {
        ins.params.insert(pi.id.to_string(), pi.default);
    }
    if let Some(obj) = p.get("params").and_then(Value::as_object) {
        for (k, v) in obj {
            let Some(pi) = info.params.iter().find(|x| x.id == k) else {
                return Err(bad("mix.insert", format!("`{plugin}` has no parameter `{k}`")));
            };
            let val = v.as_f64().map_or(pi.default, |f| (f as f32).clamp(pi.min, pi.max));
            ins.params.insert(k.clone(), val);
        }
    }
    if let Some(sl) = tr.mixer.inserts.get_mut(slot) {
        *sl = Some(ins);
    }
    Ok(json!({"track": t, "slot": slot, "plugin": info.id, "multi_mono": p.get("multi_mono").and_then(Value::as_bool).unwrap_or(false)}))
}

fn insert_remove(e: &mut Engine, p: &Value) -> Result<Value> {
    let (t, slot) = track_slot(e, p, "mix.insert_remove", INSERT_SLOTS)?;
    if let Some(tr) = e.session_mut().track_mut(t)
        && let Some(s) = tr.mixer.inserts.get_mut(slot)
    {
        *s = None;
    }
    Ok(json!({}))
}

fn insert_bypass(e: &mut Engine, p: &Value) -> Result<Value> {
    let (t, slot) = track_slot(e, p, "mix.insert_bypass", INSERT_SLOTS)?;
    let val = p.get("value").and_then(Value::as_bool);
    let s = e.session_mut();
    let ins = s
        .track_mut(t)
        .and_then(|tr| tr.mixer.inserts.get_mut(slot))
        .and_then(Option::as_mut)
        .ok_or_else(|| bad("mix.insert_bypass", "empty slot"))?;
    ins.bypass = val.unwrap_or(!ins.bypass);
    Ok(json!({"bypass": ins.bypass}))
}

fn insert_key(e: &mut Engine, p: &Value) -> Result<Value> {
    let (t, slot) = track_slot(e, p, "mix.insert_key", INSERT_SLOTS)?;
    let s = e.session_mut();
    let bus = match p.get("bus") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(n)) if n.is_empty() || n.eq_ignore_ascii_case("none") => Some(None),
        Some(Value::String(n)) => Some(Some(s.bus_by_name(n).map(|b| b.id).ok_or_else(|| bad("mix.insert_key", format!("no bus `{n}`")))?)),
        Some(Value::Number(n)) => {
            let id = n.as_u64().ok_or_else(|| bad("mix.insert_key", "bad bus id"))?;
            Some(Some(s.busses.iter().find(|b| b.id.0 == id).map(|b| b.id).ok_or_else(|| bad("mix.insert_key", format!("no bus {id}")))?))
        }
        Some(_) => return Err(bad("mix.insert_key", "`bus` must be a name, an id or null")),
    };
    let ins =
        s.track_mut(t).and_then(|tr| tr.mixer.inserts.get_mut(slot)).and_then(Option::as_mut).ok_or_else(|| bad("mix.insert_key", "empty slot"))?;
    if let Some(b) = bus {
        ins.key = b;
        ins.key_on = b.is_some();
    }
    ins.key_on = bool_or(p, "on", ins.key_on) && ins.key.is_some();
    ins.key_listen = bool_or(p, "listen", ins.key_listen) && ins.key_on;
    Ok(json!({"bus": ins.key.map(|b| b.0), "on": ins.key_on, "listen": ins.key_listen}))
}

fn insert_param(e: &mut Engine, p: &Value) -> Result<Value> {
    let (t, slot) = track_slot(e, p, "mix.insert_param", INSERT_SLOTS)?;
    let param = str_param(p, "param").ok_or_else(|| bad("mix.insert_param", "`param` required"))?.to_string();
    let value =
        p.get("value").and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad("mix.insert_param", "`value` must be a number"))? as f32;
    let s = e.session_mut();
    let ins =
        s.track_mut(t).and_then(|tr| tr.mixer.inserts.get_mut(slot)).and_then(Option::as_mut).ok_or_else(|| bad("mix.insert_param", "empty slot"))?;
    let info = super::vst3::plugin_info(&ins.plugin).ok_or_else(|| bad("mix.insert_param", "unknown plugin"))?;
    let pi =
        info.params.iter().find(|x| x.id == param).ok_or_else(|| bad("mix.insert_param", format!("`{}` has no parameter `{param}`", info.id)))?;
    let v = value.clamp(pi.min, pi.max);
    ins.params.insert(param, v);
    Ok(json!({"value": v}))
}

fn insert_automation(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.insert_automation";
    let (t, slot) = track_slot(e, p, id, INSERT_SLOTS)?;
    let ids: Vec<String> =
        p.get("params").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let enable = bool_or(p, "enable", true);
    let ins = e.session_mut().track_mut(t).and_then(|tr| tr.mixer.inserts.get_mut(slot)).and_then(Option::as_mut).ok_or_else(|| bad(id, "empty slot"))?;
    let info = super::vst3::plugin_info(&ins.plugin).ok_or_else(|| bad(id, "unknown plugin"))?;
    if let Some(x) = ids.iter().find(|x| x.as_str() != openpt_model::BYPASS_PARAM && info.param(x).is_none()) {
        return Err(bad(id, format!("`{}` has no parameter `{x}`", info.id)));
    }
    for x in ids {
        let has = ins.automated.contains(&x);
        if enable && !has {
            ins.automated.push(x);
        } else if !enable && has {
            ins.automated.retain(|a| *a != x);
        }
    }
    Ok(json!({"automated": ins.automated}))
}

fn insert_params(e: &mut Engine, p: &Value) -> Result<Value> {
    let (t, slot) = track_slot(e, p, "mix.insert_params", INSERT_SLOTS)?;
    let obj = p.get("params").and_then(Value::as_object).cloned().ok_or_else(|| bad("mix.insert_params", "`params` must be an object"))?;
    let preset = str_param(p, "preset").map(str::to_string);
    let s = e.session_mut();
    let ins = s
        .track_mut(t)
        .and_then(|tr| tr.mixer.inserts.get_mut(slot))
        .and_then(Option::as_mut)
        .ok_or_else(|| bad("mix.insert_params", "empty slot"))?;
    let info = super::vst3::plugin_info(&ins.plugin).ok_or_else(|| bad("mix.insert_params", "unknown plugin"))?;
    let mut unknown: Vec<&str> = Vec::new();
    let mut non_numeric: Vec<&str> = Vec::new();
    for (k, v) in &obj {
        if !info.params.iter().any(|x| x.id == k.as_str()) {
            unknown.push(k.as_str());
        } else if v.as_f64().is_none_or(|f| !f.is_finite()) {
            non_numeric.push(k.as_str());
        }
    }
    if !unknown.is_empty() {
        unknown.sort_unstable();
        return Err(bad("mix.insert_params", format!("`{}` has no parameter(s): {}", info.id, unknown.join(", "))));
    }
    if !non_numeric.is_empty() {
        non_numeric.sort_unstable();
        return Err(bad("mix.insert_params", format!("parameter(s) need a numeric value: {}", non_numeric.join(", "))));
    }
    let mut n = 0;
    for (k, v) in obj {
        if let (Some(pi), Some(f)) = (info.params.iter().find(|x| x.id == k), v.as_f64().filter(|f| f.is_finite())) {
            ins.params.insert(k, (f as f32).clamp(pi.min, pi.max));
            n += 1;
        }
    }
    if let Some(name) = preset {
        ins.preset = name;
    }
    Ok(json!({"set": n}))
}

fn insert_transfer(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.insert_transfer";
    let from_t = track_param(e, id, p, "from_track")?.ok_or_else(|| bad(id, "`from_track` required"))?;
    let to_t = track_param(e, id, p, "to_track")?.ok_or_else(|| bad(id, "`to_track` required"))?;
    let from = slot_of(p, "from", INSERT_SLOTS).ok_or_else(|| bad(id, "`from` slot"))?;
    let to = slot_of(p, "to", INSERT_SLOTS).ok_or_else(|| bad(id, "`to` slot"))?;
    let copy = bool_or(p, "copy", false);
    if from_t == to_t && from == to {
        return Ok(json!({}));
    }
    let s = e.session_mut();
    let mut ins = s.track(from_t).and_then(|t| t.mixer.inserts.get(from).cloned().flatten()).ok_or_else(|| bad(id, "no insert in that slot"))?;
    let dest = s.track(to_t).ok_or_else(|| bad(id, "no such track"))?;
    let basic_folder = dest.kind == openpt_model::TrackKind::Folder && dest.mixer.input == openpt_model::Route::None;
    if basic_folder || matches!(dest.kind, openpt_model::TrackKind::Midi | openpt_model::TrackKind::Vca | openpt_model::TrackKind::Video) {
        return Err(bad(id, "that track has no inserts"));
    }
    if dest.format.channels() < 2 {
        ins.multi_mono = false;
    }
    if !copy && let Some(slot) = s.track_mut(from_t).and_then(|t| t.mixer.inserts.get_mut(from)) {
        *slot = None;
    }
    if let Some(slot) = s.track_mut(to_t).and_then(|t| t.mixer.inserts.get_mut(to)) {
        *slot = Some(ins);
    }
    Ok(json!({}))
}

fn insert_move(e: &mut Engine, p: &Value) -> Result<Value> {
    let t = tracks_required(e, "mix.insert_move", p)?.first().copied().ok_or_else(|| bad("mix.insert_move", "no track"))?;
    let from = slot_of(p, "from", INSERT_SLOTS).ok_or_else(|| bad("mix.insert_move", "`from` slot"))?;
    let to = slot_of(p, "to", INSERT_SLOTS).ok_or_else(|| bad("mix.insert_move", "`to` slot"))?;
    if let Some(tr) = e.session_mut().track_mut(t) {
        tr.mixer.inserts.swap(from, to);
    }
    Ok(json!({}))
}

fn route_to_new_aux(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.route_to_new_aux";
    let tracks = tracks_required(e, id, p)?;
    let format = match str_param(p, "format") {
        Some(f) => openpt_model::ChannelFormat::from_id(f).ok_or_else(|| bad(id, format!("unknown format `{f}`")))?,
        None => openpt_model::ChannelFormat::Stereo,
    };
    let send_slot = slot_of(p, "send_slot", SEND_SLOTS);
    let s = e.session_mut();
    let taken = |s: &openpt_model::Session, n: &str| s.bus_by_name(n).is_some() || s.track_by_name(n).is_some();
    let name = match str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) if !taken(s, n) => n.to_string(),
        _ => (1..10_000).map(|k| format!("Aux {k}")).find(|n| !taken(s, n)).unwrap_or_else(|| "Aux".into()),
    };
    let bus = s.add_bus(&name, format);
    let aux = s.add_track(openpt_model::TrackKind::Aux, format, Some(&name));
    let after = tracks.iter().filter_map(|t| s.track_index(*t)).max();
    super::track::place_after(s, &[aux], after);
    if let Some(a) = s.track_mut(aux) {
        a.mixer.input = openpt_model::Route::Bus(bus);
        a.mixer.output = openpt_model::Route::Main;
    }
    for t in &tracks {
        let Some(tr) = s.track_mut(*t) else { continue };
        match send_slot {
            Some(k) => {
                if let Some(x) = tr.mixer.sends.get_mut(k) {
                    *x = Some(match x.take() {
                        Some(old) => SendSlot { target: openpt_model::Route::Bus(bus), ..old },
                        None => SendSlot::new(openpt_model::Route::Bus(bus)),
                    });
                }
            }
            None => tr.mixer.output = openpt_model::Route::Bus(bus),
        }
    }
    Ok(json!({"bus": bus, "aux": aux, "name": name}))
}

fn send(e: &mut Engine, p: &Value) -> Result<Value> {
    let bus = str_param(p, "bus").ok_or_else(|| bad("mix.send", "`bus` required"))?.to_string();
    let t = tracks_required(e, "mix.send", p)?.first().copied().ok_or_else(|| bad("mix.send", "no track"))?;
    let explicit = slot_of(p, "slot", SEND_SLOTS);
    let s = e.session_mut();
    let inf_default = s.edit.flag("pref.sends_default_inf");
    let bid = s.add_bus(&bus, openpt_model::ChannelFormat::Stereo);
    let tr = s.track_mut(t).ok_or_else(|| bad("mix.send", "no track"))?;
    let slot = explicit.or_else(|| tr.mixer.sends.iter().position(Option::is_none)).ok_or_else(|| bad("mix.send", "all send slots are in use"))?;
    let mut sl = match tr.mixer.sends.get(slot).cloned().flatten() {
        Some(old) => SendSlot { target: openpt_model::Route::Bus(bid), ..old },
        None => {
            let mut n = SendSlot::new(openpt_model::Route::Bus(bid));
            n.level_db = if inf_default { -144.0 } else { 0.0 };
            n
        }
    };
    sl.level_db = f32_or(p, "level_db", sl.level_db).clamp(-144.0, 12.0);
    sl.pre_fader = bool_or(p, "pre_fader", sl.pre_fader);
    if let Some(x) = tr.mixer.sends.get_mut(slot) {
        *x = Some(sl);
    }
    Ok(json!({"slot": slot, "bus": bid}))
}

fn send_level(e: &mut Engine, p: &Value) -> Result<Value> {
    let (t, slot) = track_slot(e, p, "mix.send_level", SEND_SLOTS)?;
    let s = e.session_mut();
    let sl = s
        .track_mut(t)
        .and_then(|tr| tr.mixer.sends.get_mut(slot))
        .and_then(Option::as_mut)
        .ok_or_else(|| bad("mix.send_level", "empty send slot"))?;
    if let Some(db) = p.get("db").and_then(Value::as_f64) {
        sl.level_db = (db as f32).clamp(-144.0, 12.0);
    }
    if let Some(v) = p.get("pan").and_then(Value::as_f64) {
        sl.pan = (v as f32).clamp(-1.0, 1.0);
    }
    if let Some(m) = p.get("mute").and_then(Value::as_bool) {
        sl.mute = m;
    }
    if let Some(m) = p.get("pre_fader").and_then(Value::as_bool) {
        sl.pre_fader = m;
    }
    if let Some(m) = p.get("follow_main_pan").and_then(Value::as_bool) {
        sl.follow_main_pan = m;
    }
    Ok(json!({"send": sl.clone()}))
}

fn lane_param(p: &Value, cmd: &str) -> Result<AutoParam> {
    str_param(p, "param").map_or(Ok(AutoParam::Volume), |s| AutoParam::parse(s).ok_or_else(|| bad(cmd, format!("unknown automation param `{s}`"))))
}

fn set_point(e: &mut Engine, p: &Value) -> Result<Value> {
    let tracks = tracks_required(e, "automation.set_point", p)?;
    let param = lane_param(p, "automation.set_point")?;
    let at = position_param(e, "automation.set_point", p, "at")?.unwrap_or(e.session().edit.selection.start).max(0);
    let from = position_param(e, "automation.set_point", p, "from")?;
    let v = p.get("value").and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad("automation.set_point", "`value` required"))? as f32;
    let s = e.session_mut();
    for t in &tracks {
        if let Some(tr) = s.track_mut(*t) {
            let lane = tr.lane_mut(&param);
            if let Some(from) = from {
                lane.clear_range(from.saturating_add(1), at);
            }
            lane.set_point(at, v);
        }
    }
    Ok(json!({"at": at, "value": v}))
}

fn move_point(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "automation.move_point";
    let t = track_param(e, id, p, "track")?.ok_or_else(|| bad(id, "`track` required"))?;
    let param = lane_param(p, id)?;
    let from = position_param(e, id, p, "from")?.ok_or_else(|| bad(id, "`from` required"))?;
    let at = position_param(e, id, p, "at")?.ok_or_else(|| bad(id, "`at` required"))?.max(0);
    let v = p.get("value").and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(id, "`value` required"))? as f32;
    let s = e.session_mut();
    let lane = s.track_mut(t).ok_or_else(|| bad(id, "no such track"))?.lane_mut(&param);
    let i = lane.points.iter().position(|q| q.at == from).ok_or_else(|| bad(id, "no point there"))?;
    let lo = if i == 0 { 0 } else { lane.points.get(i - 1).map_or(0, |q| q.at + 1) };
    let hi = lane.points.get(i + 1).map_or(Samples::MAX, |q| q.at - 1);
    let at = at.clamp(lo, hi.max(lo));
    lane.points.remove(i);
    lane.set_point(at, v);
    Ok(json!({"at": at, "value": v}))
}

fn trim_range(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "automation.trim_range";
    let t = track_param(e, id, p, "track")?.ok_or_else(|| bad(id, "`track` required"))?;
    let param = lane_param(p, id)?;
    let start = position_param(e, id, p, "start")?.ok_or_else(|| bad(id, "`start` required"))?;
    let end = position_param(e, id, p, "end")?.ok_or_else(|| bad(id, "`end` required"))?;
    let delta = p.get("delta").and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(id, "`delta` required"))? as f32;
    let tr = e.session_mut().track_mut(t).ok_or_else(|| bad(id, "no such track"))?;
    let def = current_value(tr, &param);
    if bool_or(p, "points_only", false) {
        tr.lane_mut(&param).offset_points(start, end, delta);
    } else {
        tr.lane_mut(&param).trim_range(start, end, delta, def);
    }
    Ok(json!({}))
}

fn delete_point(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "automation.delete_point";
    let t = track_param(e, id, p, "track")?.ok_or_else(|| bad(id, "`track` required"))?;
    let param = lane_param(p, id)?;
    let at = position_param(e, id, p, "at")?.ok_or_else(|| bad(id, "`at` required"))?;
    let lane = e.session_mut().track_mut(t).ok_or_else(|| bad(id, "no such track"))?.lane_mut(&param);
    let before = lane.points.len();
    lane.points.retain(|q| q.at != at);
    Ok(json!({"removed": before - lane.points.len()}))
}

fn write_range(e: &mut Engine, p: &Value) -> Result<Value> {
    let tracks = tracks_required(e, "automation.write_range", p)?;
    let param = lane_param(p, "automation.write_range")?;
    let r = range_param(e, "automation.write_range", p)?;
    let v = p.get("value").and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad("automation.write_range", "`value` required"))? as f32;
    let s = e.session_mut();
    for t in &tracks {
        if let Some(tr) = s.track_mut(*t) {
            let def = current_value(tr, &param);
            tr.lane_mut(&param).write_range(r.start, r.end, v, def);
        }
    }
    Ok(json!({}))
}

pub(crate) fn current_value(tr: &openpt_model::Track, param: &AutoParam) -> f32 {
    match param {
        AutoParam::Volume => tr.mixer.volume_db,
        AutoParam::Pan(i) => tr.mixer.pan.get(usize::from(*i)).copied().unwrap_or(0.0),
        AutoParam::Mute => f32::from(u8::from(tr.mixer.mute)),
        AutoParam::SendLevel(i) => tr.mixer.sends.get(usize::from(*i)).and_then(Option::as_ref).map_or(-144.0, |s| s.level_db),
        AutoParam::SendPan(i) => tr.mixer.sends.get(usize::from(*i)).and_then(Option::as_ref).map_or(0.0, |s| s.pan),
        AutoParam::SendMute(i) => tr.mixer.sends.get(usize::from(*i)).and_then(Option::as_ref).map_or(0.0, |s| f32::from(u8::from(s.mute))),
        AutoParam::Plugin { slot, param } => {
            tr.mixer.inserts.get(usize::from(*slot)).and_then(Option::as_ref).and_then(|i| if param == openpt_model::BYPASS_PARAM { Some(f32::from(u8::from(i.bypass))) } else { i.params.get(param).copied() }).unwrap_or(0.0)
        }
    }
}

fn clear_lane(e: &mut Engine, p: &Value) -> Result<Value> {
    let tracks = tracks_required(e, "automation.clear", p)?;
    let param = str_param(p, "param").and_then(AutoParam::parse);
    let r = range_param(e, "automation.clear", p)?;
    let (a, b) = if r.is_empty() { (i64::MIN, i64::MAX) } else { (r.start, r.end) };
    let s = e.session_mut();
    let mut n = 0;
    for t in &tracks {
        if let Some(tr) = s.track_mut(*t) {
            for l in &mut tr.automation {
                if param.as_ref().is_none_or(|x| x == &l.param) {
                    n += l.clear_range(a, b);
                }
            }
        }
    }
    Ok(json!({"removed": n}))
}

fn thin(e: &mut Engine, p: &Value, all: bool) -> Result<Value> {
    let tracks = if all { e.session().tracks.iter().map(|t| t.id).collect() } else { tracks_required(e, "automation.thin", p)? };
    let tol = f32_or(p, "tolerance", 0.1).clamp(0.0, 100.0);
    let s = e.session_mut();
    let mut n = 0;
    for t in &tracks {
        if let Some(tr) = s.track_mut(*t) {
            for l in &mut tr.automation {
                n += l.thin(tol);
            }
        }
    }
    Ok(json!({"removed": n}))
}

fn write_current(e: &mut Engine, p: &Value, all: bool) -> Result<Value> {
    let tracks = tracks_required(e, "automation.write_to_current", p)?;
    let r = range_param(e, "automation.write_to_current", p)?;
    let s = e.session_mut();
    for t in &tracks {
        if let Some(tr) = s.track_mut(*t) {
            let params: Vec<AutoParam> = if all {
                let mut v = vec![AutoParam::Volume, AutoParam::Pan(0), AutoParam::Mute];
                v.extend(tr.automation.iter().map(|l| l.param.clone()));
                v.sort();
                v.dedup();
                v
            } else {
                vec![AutoParam::parse(&tr.view).unwrap_or(AutoParam::Volume)]
            };
            for param in params {
                let v = current_value(tr, &param);
                tr.lane_mut(&param).write_range(r.start, r.end, v, v);
            }
        }
    }
    Ok(json!({}))
}

fn env_db(env: &[(i64, f32)], at: i64) -> f32 {
    let mut prev: Option<(i64, f32)> = None;
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

fn vol_to_clip_gain(e: &mut Engine, p: &Value) -> Result<Value> {
    let tracks = tracks_required(e, "automation.volume_to_clip_gain", p)?;
    let r = range_param(e, "automation.volume_to_clip_gain", p)?;
    let s = e.session_mut();
    for t in &tracks {
        let Some(tr) = s.track_mut(*t) else { continue };
        let Some(lane) = tr.lane(&AutoParam::Volume).cloned().filter(|l| !l.points.is_empty()) else { continue };
        let base = tr.mixer.volume_db;
        let delta = |at: i64| lane.value_at(at, base) - base;
        if let Some(pl) = tr.playlist_mut() {
            for c in pl.clips.iter_mut().filter(|c| c.range().overlaps(&r)) {
                let sel0 = r.start.max(c.start);
                let sel1 = r.end.min(c.end());
                if sel1 <= sel0 {
                    continue;
                }
                let rel = |at: i64| at - c.start;
                let rel0 = rel(sel0);
                let rel1 = rel(sel1);
                let mut pts: Vec<(i64, f32)> =
                    c.gain_env.iter().copied().filter(|&(at, _)| at < rel0.saturating_sub(1) || (sel1 < c.end() && at > rel1)).collect();
                if rel0 > 0 {
                    pts.push((rel0 - 1, env_db(&c.gain_env, rel0 - 1)));
                }
                pts.push((rel0, env_db(&c.gain_env, rel0) + delta(sel0)));
                for pt in lane.points.iter().filter(|pt| pt.at > sel0 && pt.at < sel1) {
                    let at = rel(pt.at);
                    pts.push((at, env_db(&c.gain_env, at) + delta(pt.at)));
                }
                for &(at, _) in c.gain_env.iter().filter(|&&(at, _)| at > rel0 && at < rel1) {
                    pts.push((at, env_db(&c.gain_env, at) + delta(c.start.saturating_add(at))));
                }
                if sel1 - 1 > sel0 {
                    let at = rel(sel1 - 1);
                    pts.push((at, env_db(&c.gain_env, at) + delta(sel1 - 1)));
                }
                if sel1 < c.end() {
                    pts.push((rel1, env_db(&c.gain_env, rel1)));
                }
                pts.sort_by_key(|p| p.0);
                let mut env = Vec::new();
                for p in pts {
                    if env.last().is_some_and(|q: &(i64, f32)| q.0 == p.0) {
                        env.pop();
                    }
                    env.push(p);
                }
                c.gain_env = env;
            }
        }
        let clips_inside = tr.clips().iter().all(|c| c.start >= r.start && c.end() <= r.end);
        let covers_all = clips_inside && lane.points.iter().all(|pt| pt.at >= r.start && pt.at < r.end);
        let lane_m = tr.lane_mut(&AutoParam::Volume);
        if covers_all {
            lane_m.clear_range(r.start, r.end);
        } else {
            let before = (r.start > 0).then(|| lane.value_at(r.start - 1, base));
            let after = lane.value_at(r.end, base);
            lane_m.clear_range(r.start, r.end);
            if let Some(v) = before {
                lane_m.set_point(r.start - 1, v);
            }
            lane_m.set_point(r.start, base);
            if r.end - 1 > r.start {
                lane_m.set_point(r.end - 1, base);
            }
            lane_m.set_point(r.end, after);
        }
    }
    Ok(json!({}))
}

fn clip_gain_to_vol(e: &mut Engine, p: &Value) -> Result<Value> {
    let tracks = tracks_required(e, "automation.clip_gain_to_volume", p)?;
    let r = range_param(e, "automation.clip_gain_to_volume", p)?;
    let s = e.session_mut();
    for t in &tracks {
        let Some(tr) = s.track_mut(*t) else { continue };
        let base = tr.mixer.volume_db;
        let mut pts = Vec::new();
        if let Some(pl) = tr.playlist_mut() {
            for c in pl.clips.iter_mut().filter(|c| c.range().overlaps(&r)) {
                if c.gain_env.is_empty() {
                    pts.push((c.start, base + c.gain_db));
                    pts.push((c.end() - 1, base + c.gain_db));
                } else {
                    for (o, db) in &c.gain_env {
                        pts.push((c.start + o, base + c.gain_db + db));
                    }
                }
                c.gain_db = 0.0;
                c.gain_env.clear();
            }
        }
        let lane = tr.lane_mut(&AutoParam::Volume);
        for (at, v) in pts {
            lane.set_point(at, v);
        }
    }
    Ok(json!({}))
}

fn copy_to_send(e: &mut Engine, p: &Value) -> Result<Value> {
    let tracks = tracks_required(e, "automation.copy_to_send", p)?;
    let slot = slot_of(p, "send", SEND_SLOTS).unwrap_or(0) as u8;
    let s = e.session_mut();
    for t in &tracks {
        if let Some(tr) = s.track_mut(*t) {
            let pts = tr.lane(&AutoParam::Volume).map(|l| l.points.clone()).unwrap_or_default();
            let v = tr.mixer.volume_db;
            let lane = tr.lane_mut(&AutoParam::SendLevel(slot));
            if pts.is_empty() {
                lane.set_point(0, v);
            }
            for pt in pts {
                lane.set_point(pt.at, pt.value);
            }
        }
    }
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use openpt_audio_io::AudioBuffer;
    use openpt_model::{ChannelFormat, Clip, SourceAudio, SourceId, TrackKind};
    use openpt_time::Range;
    use std::sync::Arc;

    #[test]
    fn inserts_copy_within_a_routing_folder_but_not_into_a_basic_one() {
        let mut e = Engine::default();
        let s = e.session_mut();
        let bus = s.add_bus("Gtrs", ChannelFormat::Stereo);
        let routing = s.add_track(TrackKind::Folder, ChannelFormat::Stereo, Some("Gtrs"));
        let basic = s.add_track(TrackKind::Folder, ChannelFormat::Stereo, Some("Box"));
        let f = s.track_mut(routing).unwrap();
        f.mixer.input = openpt_model::Route::Bus(bus);
        f.mixer.inserts[0] = Some(openpt_model::Insert::new("open_eq"));
        e.execute("mix.insert_transfer", &json!({"from_track": routing.0, "from": 0, "to_track": routing.0, "to": 1, "copy": true})).unwrap();
        let ins = &e.session().track(routing).unwrap().mixer.inserts;
        assert!(ins[0].is_some() && ins[1].is_some(), "copied next to itself");
        assert!(e.execute("mix.insert_transfer", &json!({"from_track": routing.0, "from": 0, "to_track": basic.0, "to": 0})).is_err());
    }

    #[test]
    fn a_selection_moves_by_the_same_amount_once_each_even_when_grouped() {
        let mut e = crate::demo::demo_engine();
        let (kick, snare) = (e.session().track_by_name("Kick").unwrap().id, e.session().track_by_name("Snare").unwrap().id);
        e.execute("track.group", &json!({"tracks": [kick.0, snare.0]})).unwrap();
        e.execute("mix.volume", &json!({"track": snare.0, "db": -6.0, "isolate": true})).unwrap();
        let vol = |e: &Engine, t: TrackId| e.session().track(t).unwrap().mixer.volume_db;
        let (k0, s0) = (vol(&e, kick), vol(&e, snare));
        e.execute("mix.volume", &json!({"tracks": [kick.0, snare.0], "delta_db": -2.0})).unwrap();
        assert_eq!((vol(&e, kick), vol(&e, snare)), (k0 - 2.0, s0 - 2.0));
        e.execute("mix.pan", &json!({"tracks": [kick.0], "pan": -0.5})).unwrap();
        e.execute("mix.pan", &json!({"tracks": [kick.0, snare.0], "delta": 0.25})).unwrap();
        let pan = |e: &Engine, t: TrackId| e.session().track(t).unwrap().mixer.pan[0];
        assert_eq!(pan(&e, kick), -0.25);
    }

    #[test]
    fn a_grouped_fader_moves_alone_when_isolated_or_with_groups_suspended() {
        let mut e = crate::demo::demo_engine();
        let (kick, snare) = (e.session().track_by_name("Kick").unwrap().id, e.session().track_by_name("Snare").unwrap().id);
        e.execute("track.group", &json!({"tracks": [kick.0, snare.0]})).unwrap();
        let vol = |e: &Engine, t: TrackId| e.session().track(t).unwrap().mixer.volume_db;
        let snare0 = vol(&e, snare);
        e.execute("mix.volume", &json!({"track": kick.0, "delta_db": -3.0, "isolate": true})).unwrap();
        assert_eq!(vol(&e, snare), snare0, "isolated: the group does not follow");
        e.execute("mix.suspend_groups", &json!({})).unwrap();
        e.execute("mix.volume", &json!({"track": kick.0, "delta_db": -3.0})).unwrap();
        assert_eq!(vol(&e, snare), snare0, "suspended: the group does not follow");
        e.execute("mix.suspend_groups", &json!({})).unwrap();
        e.execute("mix.volume", &json!({"track": kick.0, "delta_db": -3.0})).unwrap();
        assert_eq!(vol(&e, snare), snare0 - 3.0, "back on: the group follows");
    }

    #[test]
    fn new_aux_track_listens_to_the_bus_the_tracks_feed() {
        let mut e = crate::demo::demo_engine();
        let (kick, snare) = (e.session().track_by_name("Kick").unwrap().id, e.session().track_by_name("Snare").unwrap().id);
        let r = e.execute("mix.route_to_new_aux", &json!({"tracks": [kick.0, snare.0]})).unwrap();
        let name = r["name"].as_str().unwrap().to_string();
        let s = e.session();
        let bus = s.bus_by_name(&name).unwrap().id;
        let aux = s.track_by_name(&name).unwrap();
        assert_eq!(aux.kind, TrackKind::Aux);
        assert_eq!(aux.mixer.input, openpt_model::Route::Bus(bus));
        assert_eq!(s.track(kick).unwrap().mixer.output, openpt_model::Route::Bus(bus));
        assert_eq!(s.track(snare).unwrap().mixer.output, openpt_model::Route::Bus(bus));
        e.execute("mix.route_to_new_aux", &json!({"tracks": [kick.0], "send_slot": 2, "name": "Verb 2"})).unwrap();
        let s = e.session();
        assert!(s.track_by_name("Verb 2").is_some());
        assert!(
            matches!(&s.track(kick).unwrap().mixer.sends[2], Some(x) if x.target == openpt_model::Route::Bus(s.bus_by_name("Verb 2").unwrap().id))
        );
        e.execute("edit.undo", &json!({})).unwrap();
        assert!(e.session().track_by_name("Verb 2").is_none(), "one undo step");
    }

    #[test]
    fn inserts_move_and_copy_across_tracks() {
        let mut e = crate::demo::demo_engine();
        let (kick, snare) = (e.session().track_by_name("Kick").unwrap().id, e.session().track_by_name("Snare").unwrap().id);
        let first = e.session().track(kick).unwrap().mixer.inserts[0].clone().expect("the demo Kick has an insert");
        e.execute("mix.insert_transfer", &json!({"from_track": kick.0, "from": 0, "to_track": snare.0, "to": 3, "copy": true})).unwrap();
        assert_eq!(e.session().track(snare).unwrap().mixer.inserts[3].as_ref(), Some(&first));
        assert!(e.session().track(kick).unwrap().mixer.inserts[0].is_some(), "a copy leaves the source");
        e.execute("mix.insert_transfer", &json!({"from_track": kick.0, "from": 0, "to_track": kick.0, "to": 4})).unwrap();
        let k = &e.session().track(kick).unwrap().mixer.inserts;
        assert!(k[0].is_none() && k[4].as_ref() == Some(&first), "a move empties the source");
    }

    fn rms_db(before: &[f32], after: &[f32]) -> f32 {
        let energy = |xs: &[f32]| xs.iter().map(|x| x * x).sum::<f32>() / xs.len().max(1) as f32;
        10.0 * (energy(after).max(1e-20) / energy(before).max(1e-20)).log10()
    }

    fn tone_session(points: &[(i64, f32)]) -> (Engine, TrackId) {
        let mut s = Session::default();
        let n = 8192;
        let tone: Vec<f32> = (0..n).map(|i| (i as f32 * 0.07).sin() * 0.25).collect();
        s.pool.insert(SourceId(910), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![tone] })));
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Tone"));
        let id = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(id, "tone", SourceId(910), 0, 0, n));
        s.track_mut(t).unwrap().mixer.volume_db = -3.0;
        for &(at, value) in points {
            s.track_mut(t).unwrap().lane_mut(&AutoParam::Volume).set_point(at, value);
        }
        s.edit.selected_tracks = vec![t];
        s.edit.selection = Range::new(2048, 6144);
        (Engine::new(s), t)
    }

    fn rendered(s: &Session) -> Vec<f32> {
        openpt_mix::render_range(s, Range::new(0, 8192), 1024).into_iter().next().unwrap_or_default()
    }

    fn convert(e: &mut Engine, t: TrackId, start: i64, end: i64) {
        e.session_mut().edit.selection = Range::new(start, end);
        e.execute("automation.volume_to_clip_gain", &json!({"track": t.0, "start": start, "end": end})).unwrap();
    }

    #[test]
    fn partial_volume_to_clip_gain_keeps_each_region() {
        let points = [(0, -6.0), (2048, -6.0), (4096, -6.0), (6144, -6.0), (8192, -6.0)];
        let (mut e, t) = tone_session(&points);
        let before = rendered(e.session());
        convert(&mut e, t, 2048, 6144);
        let after = rendered(e.session());
        for (start, end) in [(0, 2048), (2048, 6144), (6144, 8192)] {
            let delta = rms_db(&before[start..end], &after[start..end]);
            assert!(delta.abs() < 0.02, "constant {start}..{end} changed by {delta} dB");
        }
        let tr = e.session().track(t).unwrap();
        assert!((tr.mixer.volume_db + 3.0).abs() < 1e-5);
        let lane = tr.lane(&AutoParam::Volume).unwrap();
        assert!((lane.value_at(100, 0.0) + 6.0).abs() < 1e-3);
        assert!((lane.value_at(3000, 0.0) + 3.0).abs() < 1e-3);
        assert!((lane.value_at(7000, 0.0) + 6.0).abs() < 1e-3);

        let (mut e, t) = tone_session(&[]);
        let before = rendered(e.session());
        convert(&mut e, t, 2048, 6144);
        let after = rendered(e.session());
        let diff = before.iter().zip(&after).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(diff < 1e-5, "no lane changed a sample by {diff}");

        let (mut e, t) = tone_session(&points);
        let before = rendered(e.session());
        convert(&mut e, t, 0, 8193);
        let after = rendered(e.session());
        let diff = before.iter().zip(&after).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(diff < 1e-4, "full selection changed a sample by {diff}");
        assert!(e.session().track(t).unwrap().lane(&AutoParam::Volume).is_none_or(|l| l.points.is_empty()));

        for points in [
            [(0, -12.0), (2048, -9.0), (4096, -6.0), (6144, -3.0), (8192, 0.0)],
            [(0, -12.0), (2048, -6.0), (4096, -12.0), (6144, -6.0), (8192, -12.0)],
        ] {
            let (mut e, t) = tone_session(&points);
            let before = rendered(e.session());
            convert(&mut e, t, 2048, 6144);
            let after = rendered(e.session());
            for (start, end) in [(0, 2048), (6144, 8192)] {
                let delta = rms_db(&before[start..end], &after[start..end]);
                assert!(delta.abs() < 0.02, "outside {start}..{end} changed by {delta} dB for {points:?}");
            }
            let delta = rms_db(&before[2048..6144], &after[2048..6144]);
            assert!(delta.abs() < 0.15, "inside changed by {delta} dB for {points:?}");
        }
    }

    #[test]
    fn lane_points_all_inside_the_selection_keep_the_outside_level() {
        let (mut e, t) = tone_session(&[(2048, -9.0), (6143, -9.0)]);
        let before = rendered(e.session());
        convert(&mut e, t, 2048, 6144);
        let after = rendered(e.session());
        for (start, end) in [(0, 2000), (2048, 6144), (6200, 8192)] {
            let delta = rms_db(&before[start..end], &after[start..end]);
            assert!(delta.abs() < 0.05, "{start}..{end} changed by {delta} dB");
        }
    }

    #[test]
    fn set_point_from_drops_the_points_in_between() {
        let (mut e, t) = tone_session(&[(100, -20.0), (150, -20.0), (200, -20.0), (250, -20.0), (300, -20.0)]);
        e.execute("automation.set_point", &json!({"track": t.0, "from": 100, "at": 250, "value": 0.0})).unwrap();
        e.execute("automation.set_point", &json!({"track": t.0, "from": 400, "at": 50, "value": -6.0})).unwrap();
        let lane = e.session().track(t).unwrap().lane(&AutoParam::Volume).unwrap();
        let points: Vec<(i64, f32)> = lane.points.iter().map(|p| (p.at, p.value)).collect();
        assert_eq!(points, [(50, -6.0), (100, -20.0), (250, 0.0), (300, -20.0)]);
    }

    #[test]
    fn clip_gain_points_inside_the_selection_are_kept() {
        let (mut e, t) = tone_session(&[(0, -6.0), (8192, -6.0)]);
        let clip = e.session().track(t).unwrap().clips()[0].id;
        e.session_mut().find_clip_mut(clip).unwrap().gain_env = vec![(3000, 0.0), (4000, -20.0), (5000, 0.0)];
        let before = rendered(e.session());
        convert(&mut e, t, 2048, 6144);
        let after = rendered(e.session());
        for (start, end) in [(2048, 3000), (3500, 4500), (5000, 6144)] {
            let delta = rms_db(&before[start..end], &after[start..end]);
            assert!(delta.abs() < 0.2, "{start}..{end} changed by {delta} dB");
        }
    }

    #[test]
    fn insert_params_rejects_unknown_and_non_numeric_keys() {
        let mut e = Engine::default();
        e.execute("track.new", &json!({})).unwrap();
        let t = e.session().tracks[0].id.0;
        e.execute("mix.insert", &json!({"track": t, "slot": 0, "plugin": "gain"})).unwrap();
        fn gain(e: &Engine) -> f32 {
            e.session().tracks[0].mixer.inserts[0].as_ref().unwrap().params["gain"]
        }

        let err = e.execute("mix.insert_params", &json!({"track": t, "slot": 0, "params": {"gain_db": -14.0}})).unwrap_err();
        assert!(err.to_string().contains("gain_db"), "{err}");

        let err = e.execute("mix.insert_params", &json!({"track": t, "slot": 0, "params": {"gain": -14.0, "width": 1.0}})).unwrap_err();
        assert!(err.to_string().contains("width"), "{err}");
        assert_eq!(gain(&e), 0.0);

        assert!(e.execute("mix.insert_params", &json!({"track": t, "slot": 0, "params": {"gain": "loud"}})).is_err());
        assert!(e.execute("mix.insert_params", &json!({"track": t, "slot": 0, "params": {"gain": null}})).is_err());
        assert_eq!(gain(&e), 0.0);

        let r = e.execute("mix.insert_params", &json!({"track": t, "slot": 0, "params": {"gain": -14.0}})).unwrap();
        assert_eq!(r["set"], 1);
        assert_eq!(gain(&e), -14.0);
        e.execute("mix.insert_params", &json!({"track": t, "slot": 0, "params": {"gain": 999.0}})).unwrap();
        assert_eq!(gain(&e), 24.0);

        e.execute("mix.insert_params", &json!({"track": t, "slot": 0, "params": {}, "preset": "Saved"})).unwrap();
        assert_eq!(e.session().tracks[0].mixer.inserts[0].as_ref().unwrap().preset, "Saved");
    }

    #[test]
    fn retargeting_a_send_keeps_its_settings() {
        let mut e = Engine::default();
        e.execute("track.new", &json!({"count": 1, "name": "Gtr", "format": "mono"})).unwrap();
        e.execute("mix.send", &json!({"track": "Gtr", "slot": 0, "bus": "Verb", "level_db": -9.0, "pre_fader": true})).unwrap();
        e.execute("mix.send_level", &json!({"track": "Gtr", "slot": 0, "pan": 0.5, "mute": true})).unwrap();
        e.execute("mix.send", &json!({"track": "Gtr", "slot": 0, "bus": "Delay"})).unwrap();
        let sl = e.session().tracks[0].mixer.sends[0].clone().unwrap();
        let delay = e.session().busses.iter().find(|b| b.name == "Delay").unwrap().id;
        assert_eq!(sl.target, openpt_model::Route::Bus(delay));
        assert_eq!((sl.level_db, sl.pan, sl.mute, sl.pre_fader), (-9.0, 0.5, true, true));
    }

    #[test]
    fn sends_are_removed_on_every_selected_track() {
        let mut e = Engine::default();
        e.execute("track.new", &json!({"count": 3, "name": "Gtr", "format": "mono"})).unwrap();
        let ids: Vec<u64> = e.session().tracks.iter().map(|t| t.id.0).collect();
        for &t in &ids {
            e.execute("mix.send", &json!({"track": t, "slot": 0, "bus": "Verb"})).unwrap();
            e.execute("mix.send", &json!({"track": t, "slot": 1, "bus": "Delay"})).unwrap();
        }
        let count = |e: &Engine, i: usize| e.session().tracks[i].mixer.sends.iter().flatten().count();

        let r = e.execute("mix.send_remove", &json!({"tracks": [ids[0], ids[1]], "slot": "b"})).unwrap();
        assert_eq!(r["removed"], 2);
        assert_eq!((count(&e, 0), count(&e, 1), count(&e, 2)), (1, 1, 2));
        assert!(e.execute("mix.send_remove", &json!({"tracks": [ids[0]]})).is_err());

        e.session_mut().edit.selected_tracks = vec![TrackId(ids[1]), TrackId(ids[2])];
        let r = e.execute("mix.sends_remove_all", &json!({})).unwrap();
        assert_eq!(r["removed"], 3);
        assert_eq!((count(&e, 0), count(&e, 1), count(&e, 2)), (1, 0, 0));
        assert!(e.undo());
        assert_eq!(count(&e, 2), 2);
    }
}
