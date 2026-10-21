use crate::winkeys::HookEvent;
pub use crate::winkeys::{Held, PadKey};
use crate::{MainWindow, SoundApp};
use egui::{Key, Modifiers};
use serde_json::json;

pub fn parse(s: &str) -> Option<(Modifiers, Key)> {
    let mut m = Modifiers::NONE;
    let mut key = None;
    for part in s.split('+') {
        match part.trim() {
            "Cmd" => m.command = true,
            "Shift" => m.shift = true,
            "Alt" => m.alt = true,
            "Ctrl" => m.ctrl = true,
            "" => {}
            k => {
                if key.is_some() {
                    return None;
                }
                key = Some(match k {
                    "Space" => Key::Space,
                    "Enter" => Key::Enter,
                    "Home" => Key::Home,
                    "End" => Key::End,
                    "Tab" => Key::Tab,
                    "[" => Key::OpenBracket,
                    "]" => Key::CloseBracket,
                    "," => Key::Comma,
                    "." => Key::Period,
                    "/" => Key::Slash,
                    "'" => Key::Quote,
                    "=" => Key::Equals,
                    other => Key::from_name(other)?,
                });
            }
        }
    }
    key.map(|k| (m, k))
}

fn mods_match(want: Modifiers, got: Modifiers, mac: bool) -> bool {
    let ctrl = if mac { want.ctrl == got.ctrl } else { !want.ctrl };
    want.command == got.command && want.shift == got.shift && want.alt == got.alt && ctrl
}

fn search_chord(m: Modifiers, mac: bool) -> bool {
    mac && m.command && m.ctrl
}

pub fn handle(app: &mut SoundApp, ctx: &egui::Context) {
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    let typing = ctx.egui_wants_keyboard_input() || app.modal_open();
    crate::winkeys::set_context(ctx);
    crate::winkeys::set_text_focus(typing);
    let hooked = crate::winkeys::drain();
    if typing {
        return;
    }
    for e in hooked {
        match e {
            HookEvent::Start { key, held } => start_chord(app, key, held),
            HookEvent::Pad { key, down, held } => pad(app, key, down, held),
        }
    }
    let events: Vec<(Key, Modifiers)> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
            .collect()
    });
    let mac = ctx.os() == egui::os::OperatingSystem::Mac;
    for (key, mods) in events {
        dispatch(app, key, mods, mac);
    }
    ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
}

fn dispatch(app: &mut SoundApp, key: Key, mods: Modifiers, mac: bool) {
    if fixed(app, key, mods, mac) {
        return;
    }
    if let Some(id) = command_for(key, mods, mac) {
        if crate::menus::invoke_shortcut_dialog(app, id) {
            return;
        }
        let _ = app.run(id, json!({}));
    } else if let Some(c) = crate::menus::UI_COMMANDS.iter().find(|c| c.3.and_then(parse).is_some_and(|(m, k)| k == key && mods_match(m, mods, mac)))
    {
        let _ = app.run(c.0, json!({}));
    }
}

pub fn start_chord(app: &mut SoundApp, key: Key, held: Held) {
    if key == Key::S && held.shift && !held.ctrl && !held.alt {
        let _ = app.run("window.search", json!({}));
        return;
    }
    let mods = Modifiers { ctrl: true, command: held.ctrl, shift: held.shift, alt: held.alt, mac_cmd: false };
    dispatch(app, key, mods, true);
}

#[derive(Debug, Default)]
pub struct Pad {
    entry: Option<String>,
    held: Vec<u8>,
}

fn shuttle_speed(held: &[u8]) -> Option<f32> {
    let has = |d| held.contains(&d);
    let way = |f: f32| {
        if has(6) {
            Some(f)
        } else if has(4) {
            Some(-f)
        } else {
            None
        }
    };
    if has(5) {
        return way(0.5);
    }
    if has(8) {
        return if has(9) {
            Some(2.0)
        } else if has(7) {
            Some(-2.0)
        } else {
            None
        };
    }
    if has(2) {
        return if has(3) {
            Some(1.0 / 16.0)
        } else if has(1) {
            Some(-1.0 / 16.0)
        } else {
            None
        };
    }
    held.last().and_then(|d| match d {
        6 => Some(1.0),
        4 => Some(-1.0),
        9 => Some(4.0),
        7 => Some(-4.0),
        3 => Some(0.25),
        1 => Some(-0.25),
        _ => None,
    })
}

pub fn pad(app: &mut SoundApp, key: PadKey, down: bool, held: Held) {
    let mode = app.ui.numeric_keypad.clone();
    if !down {
        if let PadKey::Digit(d) = key
            && app.pad.held.contains(&d)
        {
            app.pad.held.retain(|h| *h != d);
            let _ = app.run("transport.shuttle", json!({"speed": shuttle_speed(&app.pad.held).unwrap_or(0.0)}));
        }
        return;
    }
    if let PadKey::Digit(d) = key {
        if held.alt && !held.ctrl && !held.shift {
            let id = match d {
                1 => "window.time_ops",
                2 => "window.tempo_ops",
                3 => "window.midi_ops",
                5 => "clip.quantize_to_grid",
                _ => return,
            };
            let _ = app.run(id, json!({}));
            return;
        }
        if held.ctrl && held.shift && !held.alt {
            if let Some(v) = app.ui.clip_list_show.get_mut(usize::from(d).wrapping_sub(1)) {
                *v = !*v;
            }
            return;
        }
    }
    if held.ctrl || held.alt || held.shift {
        let k = match key {
            PadKey::Plus => Key::Plus,
            PadKey::Minus => Key::Minus,
            PadKey::Enter => Key::Enter,
            _ => return,
        };
        dispatch(app, k, Modifiers { ctrl: held.ctrl, command: held.ctrl, shift: held.shift, alt: held.alt, mac_cmd: false }, false);
        return;
    }
    let entry = app.pad.entry.take();
    match (key, entry) {
        (PadKey::Dot, None) => app.pad.entry = Some(String::new()),
        (PadKey::Dot, Some(n)) if !n.is_empty() => {
            let _ = app.run("markers.recall", json!({"number": n.parse::<u32>().unwrap_or(0)}));
        }
        (PadKey::Dot, Some(_)) => {}
        (PadKey::Digit(d), Some(mut n)) => {
            if n.len() < 3 {
                n.push(char::from(b'0' + d));
            }
            app.pad.entry = Some(n);
        }
        (PadKey::Star | PadKey::Slash, Some(n)) if !n.is_empty() => {
            let i = n.parse::<usize>().unwrap_or(0);
            if key == PadKey::Star {
                crate::windows::recall_configuration(app, i);
            } else {
                crate::windows::store_configuration(app, i);
            }
        }
        (PadKey::Plus | PadKey::Minus, Some(_)) if mode == "shuttle" => {
            let _ = app.run(if key == PadKey::Plus { "markers.next" } else { "markers.previous" }, json!({}));
        }
        (PadKey::Plus | PadKey::Minus, _) => {
            let _ = app.run("edit.nudge", json!({"direction": if key == PadKey::Plus { 1 } else { -1 }}));
        }
        (PadKey::Enter, _) => {
            let _ = app.run("markers.add", json!({}));
        }
        (PadKey::Digit(d), None) => match mode.as_str() {
            "classic" => app.pad.entry = Some(char::from(b'0' + d).to_string()),
            "shuttle" if d == 0 => {
                let _ = app.run("transport.play_selection", json!({"loop": true}));
            }
            "shuttle" => {
                if !app.pad.held.contains(&d) {
                    app.pad.held.push(d);
                }
                if let Some(speed) = shuttle_speed(&app.pad.held) {
                    let _ = app.run("transport.shuttle", json!({"speed": speed}));
                }
            }
            _ => {
                let id = match d {
                    0 => "transport.toggle",
                    1 => "transport.rewind",
                    2 => "transport.fast_forward",
                    3 => "transport.record",
                    4 => "options.loop_playback",
                    5 => "options.loop_record",
                    6 => "options.quickpunch",
                    7 => "options.click",
                    8 => "options.countoff",
                    _ => "options.midi_merge",
                };
                let _ = app.run(id, json!({}));
            }
        },
        (PadKey::Star | PadKey::Slash, _) => {}
    }
}

fn command_for(key: Key, mods: Modifiers, mac: bool) -> Option<&'static str> {
    openpt_engine::command_specs()
        .iter()
        .find(|c| c.shortcut.and_then(parse).is_some_and(|(m, k)| k == key && mods_match(m, mods, mac)))
        .map(|c| c.id)
}

fn fixed(app: &mut SoundApp, key: Key, m: Modifiers, mac: bool) -> bool {
    let plain = !m.command && !m.alt && !m.ctrl && !m.shift;
    match key {
        Key::Space if !m.command && !m.alt => {
            let id = if m.shift { "transport.half_speed" } else { "transport.toggle" };
            let _ = app.run(id, json!({}));
            true
        }
        Key::Space if m.command && m.shift && !m.alt => {
            let _ = app.run("transport.half_speed_record", json!({}));
            true
        }
        Key::Space if m.command => {
            let _ = app.run("transport.record", json!({}));
            true
        }
        Key::F12 if m.shift && !m.command && !m.alt => {
            let _ = app.run("transport.half_speed_record", json!({}));
            true
        }
        Key::S if search_chord(m, mac) => {
            let _ = app.run("window.search", json!({}));
            true
        }
        Key::Equals if m.command && !(mac && m.ctrl) => {
            let _ = app.run("window.toggle_mix_edit", json!({}));
            true
        }
        Key::Enter if plain => {
            let _ = app.run("markers.add", json!({}));
            true
        }
        Key::Enter if !m.alt && (m.command || m.shift) => {
            let id = match (m.command, m.shift) {
                (true, false) => "transport.go_to_end",
                (false, true) => "edit.extend_to_start",
                _ => "edit.extend_to_end",
            };
            let _ = app.run(id, json!({}));
            true
        }
        Key::Space if m.alt && !m.command && !m.shift => {
            let _ = app.run("options.transport_online", json!({}));
            true
        }
        Key::Escape if plain && app.ui.window == MainWindow::Edit => {
            let tools = ["zoom", "trim", "selector", "grabber", "scrubber", "pencil"];
            let now = app.engine.session().edit.tool.id();
            let next = tools.iter().position(|t| *t == now).map_or(0, |i| (i + 1) % tools.len());
            let _ = app.run("edit.tool", json!({"tool": tools[next]}));
            true
        }
        Key::ArrowUp | Key::ArrowDown if plain && app.ui.window == MainWindow::Edit => {
            let id = if key == Key::ArrowDown { "edit.insertion_to_selection_start" } else { "edit.insertion_to_selection_end" };
            let _ = app.run(id, json!({}));
            true
        }
        Key::ArrowUp | Key::ArrowDown if m.shift && !m.command && !m.alt && !m.ctrl => {
            let _ = app.run("track.playlist_step", json!({"direction": if key == Key::ArrowUp { -1 } else { 1 }}));
            true
        }
        Key::E if m.alt && m.shift && !m.command => {
            let _ = app.run("view.zoom_toggle", json!({"keep": true}));
            true
        }
        Key::Minus if m.alt && !m.shift && !m.command => {
            let _ = app.run("track.view_toggle", json!({}));
            true
        }
        Key::Backspace | Key::Delete if plain => {
            let _ = app.run("edit.clear", json!({}));
            true
        }
        Key::R if m.command && m.shift && !m.alt => {
            crate::menus::invoke_shortcut_dialog(app, "clip.rename");
            true
        }
        Key::Plus | Key::Equals | Key::Minus if m.shift && !m.command && !m.ctrl && app.ui.window == MainWindow::Edit => {
            let step = if key == Key::Minus { -1 } else { 1 };
            let _ = app.run(if m.alt { "view.nudge" } else { "view.grid" }, json!({"step": step}));
            true
        }
        Key::F12 if plain => {
            let _ = app.run("transport.record", json!({}));
            true
        }
        Key::F1 | Key::F2 | Key::F3 | Key::F4 if plain => {
            edit_mode_key(app, key);
            true
        }
        Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 if m.alt && !m.command && !m.ctrl && !m.shift => {
            let f = match key {
                Key::Num1 => Key::F1,
                Key::Num2 => Key::F2,
                Key::Num3 => Key::F3,
                _ => Key::F4,
            };
            edit_mode_key(app, f);
            true
        }
        Key::Backtick if plain => {
            cycle_edit_mode(app);
            true
        }
        Key::F5 | Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 if plain => {
            let tool = match key {
                Key::F5 => "zoom",
                Key::F6 => "trim",
                Key::F7 => "selector",
                Key::F8 => "grabber",
                Key::F9 => "scrubber",
                _ => "pencil",
            };
            let ed = &app.engine.session().edit;
            let all: &[&str] = match tool {
                "trim" => &["standard", "tce", "scrub", "loop"],
                "grabber" => &["time", "separation", "object"],
                _ => &[],
            };
            if ed.tool.id() == tool && !all.is_empty() {
                let now = openpt_engine::cmd::tool_variant(ed, ed.tool);
                let next = all[(all.iter().position(|v| *v == now).unwrap_or(0) + 1) % all.len()];
                let _ = app.run("edit.tool", json!({"tool": tool, "variant": next}));
            } else {
                let _ = app.run("edit.tool", json!({"tool": tool}));
            }
            true
        }
        Key::Plus | Key::Minus if !m.command && app.ui.window == MainWindow::Edit => {
            let dir = if key == Key::Plus { 1 } else { -1 };
            let _ = app.run("edit.nudge", json!({"direction": dir}));
            true
        }
        Key::Tab if !(m.command && (mac || m.alt)) && !(mac && m.ctrl) => {
            let _ = app.run("edit.tab", json!({"back": m.alt || (!mac && m.ctrl), "extend": m.shift}));
            true
        }
        Key::OpenBracket | Key::CloseBracket if mods_match(Modifiers::COMMAND | Modifiers::ALT, m, mac) => {
            let factor = if key == Key::CloseBracket { 2.0 } else { 0.5 };
            let _ = app.run("view.waveform_zoom", json!({"factor": factor}));
            true
        }
        Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5
            if mods_match(Modifiers::COMMAND | Modifiers::ALT | Modifiers::CTRL, m, mac) =>
        {
            let _ = app.run("clip.rating", json!({"rating": digit(key)}));
            true
        }
        Key::Num1 | Key::Num2 | Key::Num3 if mods_match(Modifiers::COMMAND | Modifiers::ALT, m, mac) => {
            let focus = ["commands", "clips", "groups"][usize::from(digit(key)) - 1];
            let _ = app.run("options.keyboard_focus", json!({"focus": focus}));
            true
        }
        Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 if mods_match(Modifiers::COMMAND | Modifiers::CTRL, m, mac) => {
            let _ = app.run("view.zoom_preset", json!({"preset": digit(key)}));
            true
        }
        Key::ArrowUp | Key::ArrowDown if m.ctrl && (m.alt || !m.shift) => {
            let s = app.engine.session();
            let tracks = crate::edit_window::height_targets(s, m, &s.edit.selected_tracks);
            let id = if key == Key::ArrowUp { "track.height_up" } else { "track.height_down" };
            let _ = app.run(id, if tracks.is_empty() { json!({}) } else { json!({"tracks": tracks}) });
            true
        }
        Key::Semicolon | Key::Colon if mac && m.ctrl && m.shift && !m.alt && !m.command => {
            let _ = app.run("edit.extend_selection_down", json!({}));
            true
        }
        Key::ArrowUp | Key::ArrowDown if !mac && m.ctrl && m.shift && !m.alt => {
            let id = if key == Key::ArrowUp { "clip.gain_nudge_up" } else { "clip.gain_nudge_down" };
            let _ = app.run(id, json!({}));
            true
        }
        k if !m.command
            && !m.alt
            && !m.ctrl
            && app.ui.window == MainWindow::Edit
            && app.engine.session().edit.keyboard_focus == "commands"
            && commands_focus(app, k, m.shift) =>
        {
            true
        }
        k if !m.command && !m.alt && !m.ctrl && app.engine.session().edit.keyboard_focus == "clips" && clip_list_focus(app, k) => true,
        k if plain && app.engine.session().edit.keyboard_focus == "groups" && groups_focus(app, k) => true,
        Key::R | Key::T if plain => {
            let id = if key == Key::T { "view.zoom_in" } else { "view.zoom_out" };
            let _ = app.run(id, json!({}));
            true
        }
        _ => false,
    }
}

fn edit_mode_key(app: &mut SoundApp, pressed: Key) {
    let ctx = app.docking_context.clone();
    let held = |k: Key| ctx.as_ref().is_some_and(|c| c.input(|i| i.key_down(k)));
    let (key, snap) = match [Key::F1, Key::F2, Key::F3].into_iter().find(|k| *k != pressed && held(*k)) {
        Some(k) if pressed == Key::F4 => (k, true),
        _ => (pressed, pressed != Key::F4 && held(Key::F4)),
    };
    let ed = &app.engine.session().edit;
    let keep_grid = ed.edit_mode.is_grid() && ed.flag("options.edit_tool_mode_keyboard_lock");
    let flipped = egui::Id::new("edit_mode_key_grid_before_f4");
    let before = (ed.edit_mode.is_grid() && !keep_grid).then(|| if ed.flag(openpt_engine::edit::RELATIVE_GRID) { "relative_grid" } else { "grid" });
    let restore = match &ctx {
        Some(c) if pressed == Key::F4 => {
            c.data_mut(|d| d.insert_temp(flipped, before));
            None
        }
        Some(c) if snap => c.data_mut(|d| d.remove_temp::<Option<&'static str>>(flipped)).flatten(),
        _ => None,
    };
    let mode = match key {
        Key::F1 => "shuffle",
        Key::F2 => "slip",
        Key::F3 => "spot",
        _ if keep_grid => return,
        _ => grid_mode(app),
    };
    if let Some(grid) = restore {
        let _ = app.run("edit.mode", json!({"mode": grid}));
    }
    let _ = app.run("edit.mode", json!({"mode": mode, "snap_to_grid": snap}));
}

pub(crate) fn grid_mode(app: &SoundApp) -> &'static str {
    use openpt_model::EditMode;
    let ed = &app.engine.session().edit;
    match ed.edit_mode {
        EditMode::Grid => "relative_grid",
        EditMode::GridRelative => "grid",
        _ if ed.flag(openpt_engine::edit::RELATIVE_GRID) => "relative_grid",
        _ => "grid",
    }
}

fn cycle_edit_mode(app: &mut SoundApp) {
    use openpt_model::EditMode;
    let ed = &app.engine.session().edit;
    let mode = match ed.edit_mode {
        EditMode::Shuffle => "slip",
        EditMode::Slip => "spot",
        EditMode::Spot => grid_mode(app),
        _ if ed.flag(openpt_engine::edit::SHUFFLE_LOCK) => "slip",
        _ => "shuffle",
    };
    let _ = app.run("edit.mode", json!({"mode": mode}));
}

fn commands_focus(app: &mut SoundApp, key: Key, shift: bool) -> bool {
    let id = match key {
        Key::Z if shift => "edit.redo",
        Key::P if shift => "edit.extend_selection_up",
        Key::Semicolon | Key::Colon if shift => "edit.extend_selection_down",
        _ if shift => return false,
        Key::A => "edit.trim_start_to_insertion",
        Key::S => "edit.trim_end_to_insertion",
        Key::D => "edit.fade_to_start",
        Key::G => "edit.fade_to_end",
        Key::F => "edit.fades_create",
        Key::Comma | Key::Period | Key::M | Key::Slash => {
            let direction = if matches!(key, Key::Period | Key::Slash) { 1 } else { -1 };
            let _ = app.run("edit.nudge", json!({"direction": direction, "next": matches!(key, Key::M | Key::Slash)}));
            return true;
        }
        Key::Num0 => "edit.timeline_to_selection",
        Key::O => "edit.selection_to_timeline",
        Key::OpenBracket => "transport.play_selection",
        Key::CloseBracket => "edit.play_timeline",
        Key::B => "edit.separate",
        Key::X => "edit.cut",
        Key::C => "edit.copy",
        Key::V => "edit.paste",
        Key::Z => "edit.undo",
        Key::N => "options.insertion_follows_playback",
        Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 => {
            let _ = app.run("view.zoom_preset", json!({"preset": digit(key)}));
            return true;
        }
        Key::P => {
            move_selection_track(app, -1);
            return true;
        }
        Key::Semicolon => {
            move_selection_track(app, 1);
            return true;
        }
        Key::L | Key::Quote => {
            tab_clip(app, key == Key::L);
            return true;
        }
        Key::E => {
            crate::edit_window::zoom_toggle(app);
            return true;
        }
        _ => return false,
    };
    let _ = app.run(id, json!({}));
    true
}

fn clip_list_focus(app: &mut SoundApp, key: Key) -> bool {
    let items = crate::panels::clip_list_items(app);
    let pick = |app: &mut SoundApp, i: usize| {
        if let Some((id, _, whole, _, _)) = items.get(i) {
            let _ = app.run("edit.select", if *whole { json!({"sources": [id]}) } else { json!({"clips": [id]}) });
        }
    };
    if matches!(key, Key::ArrowUp | Key::ArrowDown) {
        let cur = items.iter().position(|i| i.4);
        let next = match (cur, key) {
            (None, _) => 0,
            (Some(c), Key::ArrowUp) => c.saturating_sub(1),
            (Some(c), _) => (c + 1).min(items.len().saturating_sub(1)),
        };
        pick(app, next);
        return true;
    }
    let name = key.name();
    if name.chars().count() != 1 || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    let Some(ctx) = app.docking_context.clone() else { return false };
    let id = egui::Id::new("clip_list_typeahead");
    let now = ctx.input(|i| i.time);
    let (mut typed, at): (String, f64) = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    if now - at > 1.0 {
        typed.clear();
    }
    typed.push_str(&name.to_lowercase());
    ctx.data_mut(|d| d.insert_temp(id, (typed.clone(), now)));
    if let Some(i) = items.iter().position(|i| i.1.to_lowercase().starts_with(&typed)) {
        pick(app, i);
    }
    true
}

fn groups_focus(app: &mut SoundApp, key: Key) -> bool {
    let name = key.name().to_lowercase();
    let Some(group) = app.engine.session().groups.iter().find(|g| g.letter.to_string() == name).map(|g| g.id.0) else {
        return false;
    };
    let _ = app.run("track.group_toggle", json!({"group": group}));
    true
}

fn move_selection_track(app: &mut SoundApp, dir: i64) {
    let s = app.engine.session();
    let visible: Vec<u64> = s.tracks.iter().filter(|t| !t.hidden).map(|t| t.id.0).collect();
    let Some(cur) = s.edit.selected_tracks.first().map(|t| t.0) else { return };
    let Some(i) = visible.iter().position(|t| *t == cur) else { return };
    let j = (i as i64 + dir).clamp(0, visible.len() as i64 - 1) as usize;
    if let Some(t) = visible.get(j) {
        let sel = s.edit.selection;
        let _ = app.run("edit.select", json!({"tracks": [t], "start": sel.start, "end": sel.end, "exact": true}));
    }
}

fn tab_clip(app: &mut SoundApp, back: bool) {
    let s = app.engine.session();
    let at = s.edit.selection.start;
    let mut edges: Vec<i64> =
        s.tracks.iter().filter(|t| s.edit.selected_tracks.contains(&t.id)).flat_map(|t| t.clips().iter().flat_map(|c| [c.start, c.end()])).collect();
    edges.sort_unstable();
    let next = if back { edges.iter().rev().find(|e| **e < at).copied() } else { edges.iter().find(|e| **e > at).copied() };
    if let Some(n) = next {
        let _ = app.run("transport.locate", json!({"at": n}));
    }
}

fn digit(key: Key) -> u8 {
    key.name().parse().unwrap_or(0)
}

pub fn shortcut_label(s: &str, mac: bool) -> String {
    let has = |m: &str| s.split('+').any(|p| p == m);
    if mac || (!has("Cmd") && !has("Ctrl")) || PC_CTRL_CHORDS.contains(&s) {
        return s.to_string();
    }
    if has("Ctrl") && !cfg!(windows) {
        return String::new();
    }
    s.split('+')
        .map(|p| match p {
            "Cmd" => "Ctrl",
            "Ctrl" => "Win",
            p => p,
        })
        .collect::<Vec<_>>()
        .join("+")
}

const PC_CTRL_CHORDS: [&str; 4] = ["Ctrl+Up", "Ctrl+Down", "Ctrl+Shift+Up", "Ctrl+Shift+Down"];

#[cfg(test)]
mod label_tests {
    use super::shortcut_label;

    #[test]
    fn cmd_becomes_ctrl_off_mac() {
        assert_eq!(shortcut_label("Cmd+N", false), "Ctrl+N");
        assert_eq!(shortcut_label("Cmd+Shift+W", false), "Ctrl+Shift+W");
        assert_eq!(shortcut_label("Cmd+Alt+B", false), "Ctrl+Alt+B");
        assert_eq!(shortcut_label("Cmd+,", false), "Ctrl+,");
        let win = |s: &str| if cfg!(windows) { s.to_string() } else { String::new() };
        assert_eq!(shortcut_label("Cmd+Ctrl+S", false), win("Ctrl+Win+S"));
        assert_eq!(shortcut_label("Shift+R", false), "Shift+R");
        assert_eq!(shortcut_label("", false), "");
        assert_eq!(shortcut_label("Ctrl+Up", false), "Ctrl+Up");
        assert_eq!(shortcut_label("Ctrl+Shift+Down", false), "Ctrl+Shift+Down");
        assert_eq!(shortcut_label("Ctrl+Shift+P", false), win("Win+Shift+P"));
        assert_eq!(shortcut_label("Ctrl+Alt+L", false), win("Win+Alt+L"));
    }

    #[test]
    fn unchanged_on_mac() {
        for s in ["Cmd+N", "Cmd+Shift+W", "Cmd+Alt+B", "Shift+R", "Cmd+Ctrl+S"] {
            assert_eq!(shortcut_label(s, true), s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{command_for, mods_match, parse, search_chord};
    use egui::{Key, Modifiers};

    const PC_CTRL: Modifiers = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    const MAC_CMD: Modifiers = Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE };

    fn want(s: &str) -> Modifiers {
        parse(s).map(|(m, _)| m).unwrap()
    }

    #[test]
    fn ctrl_is_cmd_off_macos() {
        assert!(mods_match(want("Cmd+S"), PC_CTRL, false));
        assert!(mods_match(want("Cmd+Shift+N"), Modifiers { shift: true, ..PC_CTRL }, false));
        assert!(mods_match(want("Cmd+X"), PC_CTRL, false));
        assert!(!mods_match(want("Cmd+Ctrl+X"), PC_CTRL, false));
        assert!(!mods_match(want("Cmd+S"), Modifiers::NONE, false));
        assert!(!search_chord(PC_CTRL, false));
    }

    #[test]
    fn cmd_and_ctrl_stay_apart_on_macos() {
        assert!(mods_match(want("Cmd+S"), MAC_CMD, true));
        assert!(!mods_match(want("Cmd+S"), Modifiers::CTRL, true));
        assert!(mods_match(want("Cmd+Ctrl+X"), Modifiers { ctrl: true, ..MAC_CMD }, true));
        assert!(!mods_match(want("Cmd+X"), Modifiers { ctrl: true, ..MAC_CMD }, true), "Cmd+Ctrl+X is not Cut");
        assert!(!search_chord(MAC_CMD, true));
        assert!(search_chord(Modifiers { ctrl: true, ..MAC_CMD }, true));
    }

    #[test]
    fn shift_cmd_k_prompts_for_a_folder_to_export_clips() {
        assert_eq!(command_for(Key::K, Modifiers { shift: true, ..MAC_CMD }, true), Some("file.export_clips"));
        assert_eq!(command_for(Key::K, Modifiers { shift: true, ..PC_CTRL }, false), Some("file.export_clips"));
        assert!(crate::dialogs::takes_path("file.export_clips"));
    }
}
