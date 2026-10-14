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
    if ctx.egui_wants_keyboard_input() || app.modal_open() {
        return;
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
        if fixed(app, key, mods, mac) {
            continue;
        }
        if let Some(id) = command_for(key, mods, mac) {
            if crate::menus::invoke_shortcut_dialog(app, id) {
                continue;
            }
            let _ = app.run(id, json!({}));
        } else if let Some(c) =
            crate::menus::UI_COMMANDS.iter().find(|c| c.3.and_then(parse).is_some_and(|(m, k)| k == key && mods_match(m, mods, mac)))
        {
            let _ = app.run(c.0, json!({}));
        }
    }
    ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
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
        Key::Space if m.command => {
            let _ = app.run("transport.record", json!({}));
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
            let _ = app.run("edit.tool", json!({"tool": tool}));
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
        Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 if mods_match(Modifiers::COMMAND | Modifiers::ALT, m, mac) => {
            let _ = app.run("clip.rating", json!({"rating": digit(key)}));
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
    if has("Ctrl") {
        return String::new();
    }
    s.split('+').map(|p| if p == "Cmd" { "Ctrl" } else { p }).collect::<Vec<_>>().join("+")
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
        assert_eq!(shortcut_label("Shift+R", false), "Shift+R");
        assert_eq!(shortcut_label("Cmd+Ctrl+S", false), "");
        assert_eq!(shortcut_label("", false), "");
        assert_eq!(shortcut_label("Ctrl+Up", false), "Ctrl+Up");
        assert_eq!(shortcut_label("Ctrl+Shift+Down", false), "Ctrl+Shift+Down");
        assert_eq!(shortcut_label("Ctrl+Shift+P", false), "");
        assert_eq!(shortcut_label("Ctrl+Alt+L", false), "");
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
