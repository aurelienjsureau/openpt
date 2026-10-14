use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::Harness;
use openpt_ui_egui::{Services, SoundApp};

fn chord(spec: &str) -> (Key, Modifiers) {
    let mut m = Modifiers::NONE;
    let mut key = None;
    for part in spec.split('+') {
        match part {
            "Ctrl" => {
                m.ctrl = true;
                m.command = true;
            }
            "Alt" => m.alt = true,
            "Shift" => m.shift = true,
            "" => key = Some(Key::Plus),
            k => {
                key = Some(match k {
                    "Space" => Key::Space,
                    "Enter" => Key::Enter,
                    "Tab" => Key::Tab,
                    "Esc" => Key::Escape,
                    "Backspace" => Key::Backspace,
                    "Up" => Key::ArrowUp,
                    "Down" => Key::ArrowDown,
                    "Left" => Key::ArrowLeft,
                    "Right" => Key::ArrowRight,
                    "[" => Key::OpenBracket,
                    "]" => Key::CloseBracket,
                    "/" => Key::Slash,
                    "\\" => Key::Backslash,
                    "." => Key::Period,
                    "," => Key::Comma,
                    ";" => Key::Semicolon,
                    "'" => Key::Quote,
                    "=" => Key::Equals,
                    "-" => Key::Minus,
                    "`" => Key::Backtick,
                    other => Key::from_name(other).unwrap_or_else(|| panic!("key {other}")),
                })
            }
        }
    }
    (key.unwrap_or_else(|| panic!("no key in {spec}")), m)
}

const GUIDE: &[(&str, &str, &str)] = &[
    ("Space", "transport.toggle", "Start/stop playback"),
    ("Shift+Space", "transport.half_speed", "Half-speed playback"),
    ("Ctrl+J", "options.transport_online", "Enable/disable online playback"),
    ("Alt+Space", "options.transport_online", "Enable/disable online playback"),
    ("Alt+[", "transport.play_selection", "Play Edit selection"),
    ("Alt+]", "edit.play_timeline", "Play Timeline selection"),
    ("Ctrl+Shift+L", "options.loop_playback", "Loop playback toggle"),
    ("Ctrl+Space", "transport.record", "Start record"),
    ("F12", "transport.record", "Start record"),
    ("Ctrl+.", "transport.discard_take", "Stop record and discard the take"),
    ("Alt+L", "options.loop_record", "Loop Record toggle"),
    ("Ctrl+Shift+T", "options.trackpunch", "TrackPunch toggle"),
    ("Ctrl+Shift+P", "options.quickpunch", "QuickPunch toggle"),
    ("Ctrl+K", "options.pre_post_roll", "Enable or disable pre/post-roll"),
    ("F1", "edit.mode", "Shuffle"),
    ("F2", "edit.mode", "Slip"),
    ("F3", "edit.mode", "Spot"),
    ("F4", "edit.mode", "Grid"),
    ("Alt+1", "edit.mode", "Shuffle"),
    ("Alt+4", "edit.mode", "Grid"),
    ("F5", "edit.tool", "Zoomer"),
    ("F6", "edit.tool", "Trimmer"),
    ("F7", "edit.tool", "Selector"),
    ("F8", "edit.tool", "Grabber"),
    ("F9", "edit.tool", "Scrubber"),
    ("F10", "edit.tool", "Pencil"),
    ("`", "edit.mode", "Cycle through Edit modes"),
    ("Esc", "edit.tool", "Cycle through Edit tools"),
    ("Ctrl+Alt+Tab", "options.tab_to_transient", "Tab to Transients"),
    ("Tab", "edit.tab", "Next clip boundary"),
    ("Ctrl+Tab", "edit.tab", "Previous clip boundary"),
    ("Shift+Tab", "edit.tab", "Extend to next clip boundary"),
    ("Ctrl+Shift+Tab", "edit.tab", "Extend to previous clip boundary"),
    ("Ctrl+Enter", "transport.go_to_end", "Go to end of session"),
    ("Shift+Enter", "edit.extend_to_start", "Extend selection to start of session"),
    ("Ctrl+Shift+Enter", "edit.extend_to_end", "Extend selection to end of session"),
    ("Shift+/", "options.link_timeline_edit", "Link Timeline and Edit selection"),
    ("Down", "edit.insertion_to_selection_start", "Insertion to the start of the Edit selection"),
    ("Up", "edit.insertion_to_selection_end", "Insertion to the end of the Edit selection"),
    ("Alt+Shift+E", "view.zoom_toggle", "Cancel Zoom Toggle"),
    ("Shift+R", "mix.record_arm", "Record enable tracks with the Edit selection"),
    ("Shift+I", "mix.input_monitor", "Input monitor tracks with the Edit selection"),
    ("Shift+S", "mix.solo", "Solo tracks with the Edit selection"),
    ("Shift+M", "mix.mute", "Mute tracks with the Edit selection"),
    ("Shift+=", "view.grid", "Increase the Grid value"),
    ("Shift+-", "view.grid", "Decrease the Grid value"),
    ("Shift+Alt+=", "view.nudge", "Increase the Nudge value"),
    ("Shift+Up", "track.playlist_step", "Previous playlist"),
    ("Alt+-", "track.view_toggle", "Toggle Track View on all tracks"),
    ("Ctrl+]", "view.zoom_in", "Horizontal zoom in"),
    ("Ctrl+[", "view.zoom_out", "Horizontal zoom out"),
    ("Ctrl+Alt+]", "view.waveform_zoom", "Zoom in vertically on audio"),
    ("Ctrl+Alt+[", "view.waveform_zoom", "Zoom out vertically on audio"),
    ("Alt+A", "view.zoom_fit", "Horizontal zoom to show all clips"),
    ("Alt+F", "view.zoom_to_selection", "Fill window with selection"),
    ("Ctrl+Alt+H", "automation.copy_to_send", "Copy Automation to Send"),
    ("Ctrl+Alt+T", "automation.thin", "Thin Automation"),
    ("Ctrl+/", "automation.write_to_current", "Write Automation to Current Parameter"),
    ("Ctrl+Alt+/", "automation.write_to_all", "Write Automation to All Enabled Parameters"),
    ("Backspace", "edit.clear", "Delete"),
    ("Ctrl+F", "edit.fades_create", "Create Fades"),
    ("Ctrl+N", "session.new", "New Session"),
    ("Ctrl+O", "session.open", "Open Session"),
    ("Ctrl+Shift+W", "session.close", "Close Session"),
    ("Ctrl+S", "session.save", "Save Session"),
    ("Ctrl+Alt+B", "file.bounce_mix", "Bounce to Disk"),
    ("Shift+Alt+I", "file.import_session_data", "Import Session Data"),
    ("Ctrl+Shift+I", "file.import_audio", "Import Audio"),
    ("Ctrl+Alt+I", "file.import_midi", "Import MIDI"),
    ("Ctrl+Z", "edit.undo", "Undo"),
    ("Ctrl+Shift+Z", "edit.redo", "Redo"),
    ("Ctrl+Alt+Z", "edit.restore_last_selection", "Restore Last Selection"),
    ("Ctrl+X", "edit.cut", "Cut"),
    ("Ctrl+C", "edit.copy", "Copy"),
    ("Ctrl+V", "edit.paste", "Paste"),
    ("Ctrl+B", "edit.clear", "Clear"),
    ("Ctrl+A", "edit.select_all", "Select All"),
    ("Alt+M", "edit.paste_merge_midi", "Paste Special Merge"),
    ("Ctrl+Alt+V", "edit.paste_repeat_to_fill", "Paste Special Repeat to Fill Selection"),
    ("Alt+Shift+6", "edit.selection_to_timeline", "Change Timeline Selection to Edit Selection"),
    ("Alt+Shift+5", "edit.timeline_to_selection", "Change Edit Selection to Match Timeline Selection"),
    ("Ctrl+D", "edit.duplicate", "Duplicate selection"),
    ("Alt+R", "edit.repeat", "Repeat selection"),
    ("Alt+H", "edit.shift", "Shift selection"),
    ("Ctrl+Shift+E", "edit.insert_silence", "Insert Silence"),
    ("Ctrl+T", "edit.trim_to_selection", "Trim Clip to Selection"),
    ("Alt+Shift+7", "edit.trim_start_to_insertion", "Trim Clip Start to Insertion"),
    ("Alt+Shift+8", "edit.trim_end_to_insertion", "Trim Clip End to Insertion"),
    ("Ctrl+E", "edit.separate", "Separate Clip at Selection"),
    ("Ctrl+H", "edit.heal", "Heal Separation"),
    ("Alt+Shift+3", "edit.consolidate", "Consolidate Clip"),
    ("Ctrl+M", "edit.mute_clips", "Mute/Unmute Clips"),
    ("Alt+Shift+Down", "edit.copy_to_new_playlist", "Copy the selection to a new playlist"),
    ("Alt+Shift+N", "edit.move_to_new_playlist", "Move the selection to a new playlist"),
    ("Ctrl+U", "edit.strip_silence", "Strip Silence"),
    ("Ctrl+Alt+M", "window.narrow_mix", "Narrow Mix"),
    ("Alt+7", "window.universe", "Universe"),
    ("Alt+6", "window.clip_effects_dock", "Clip Effects"),
    ("Ctrl+Shift+N", "track.new", "New Track"),
    ("Ctrl+G", "track.group", "Group Tracks"),
    ("Alt+Shift+D", "track.duplicate", "Duplicate Tracks"),
    ("Alt+Shift+C", "track.commit", "Commit Track"),
    ("Ctrl+Alt+Shift+B", "track.bounce", "Bounce Track"),
    ("Alt+K", "track.set_record_to_input_only", "Set Record Tracks to Input Only"),
    ("Ctrl+Alt+F", "track.scroll_to", "Scroll to Track"),
    ("Alt+C", "track.clear_clip_indicators", "Clear all Clip indicators"),
    ("Ctrl+L", "clip.edit_lock", "Lock/Unlock Clips"),
    ("Alt+Shift+F", "clip.bring_to_front", "Bring to Front"),
    ("Alt+Shift+B", "clip.send_to_back", "Send to Back"),
    ("Ctrl+Alt+G", "clip.group", "Group Clips"),
    ("Ctrl+Alt+U", "clip.ungroup", "Ungroup Clips"),
    ("Ctrl+Alt+R", "clip.regroup", "Regroup Clips"),
    ("Ctrl+Alt+L", "clip.loop", "Loop Clip"),
    ("Ctrl+R", "clip.capture", "Capture Clip"),
    ("Ctrl+Alt+Shift+R", "clip.rename", "Rename Clip"),
    ("Ctrl+,", "clip.identify_sync_point", "Identify Sync Point"),
    ("Ctrl+0", "clip.quantize_to_grid", "Quantize Clip to Grid"),
    ("Alt+5", "clip.elastic_properties", "Elastic Properties"),
    ("Alt+0", "event.quantize", "Quantize"),
    ("Alt+P", "event.change_duration", "Change Duration"),
    ("Alt+T", "event.transpose", "Transpose"),
    ("Alt+Y", "event.select_split_notes", "Select/Split Notes"),
    ("Ctrl+I", "event.identify_beat", "Identify Beat"),
    ("Ctrl+Shift+.", "event.all_notes_off", "All MIDI Notes Off"),
    ("Ctrl+=", "window.toggle_mix_edit", "Toggle Mix and Edit windows"),
    ("Ctrl+Alt+J", "window.configurations", "Window Configuration List"),
    ("Alt+'", "window.task_manager", "Task Manager"),
    ("Alt+I", "window.workspace", "Workspace"),
    ("Alt+J", "window.workspace_front", "Workspaces to Front"),
    ("Alt+=", "window.event_list", "MIDI Event List"),
    ("Ctrl+Shift+F", "clip_list.find", "Find Clip"),
    ("Ctrl+Shift+A", "clip_list.select_all", "Select All Clips in Clip List"),
    ("Ctrl+Shift+U", "clip_list.select_unused", "Select Unused Clips"),
    ("Ctrl+Shift+B", "clip.clear", "Clear selected Clips"),
    ("Ctrl+Shift+R", "clip.rename", "Rename selected Clips"),
    ("Ctrl+Shift+K", "file.export_clips", "Export Clips as Files"),
];

fn reaches(spec: &str, want: &str) -> Result<(), String> {
    let mut app = SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default());
    let kick = app.engine.session().track_by_name("Kick").map(|t| (t.id.0, t.clips()[0].id.0));
    if let Some((t, c)) = kick {
        let _ = app.run("edit.select", serde_json::json!({"tracks": [t], "clips": [c]}));
    }
    app.command_log.clear();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h.state_mut().command_log.clear();
    let (key, modifiers) = chord(spec);
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.event(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.run_steps(2);
    let log: Vec<String> = h.state().command_log.iter().cloned().collect();
    if log.iter().any(|c| c == want) { Ok(()) } else { Err(format!("{spec:>18} → {want:<36} got {log:?}")) }
}

#[test]
fn every_guide_shortcut_reaches_its_command() {
    let failures: Vec<String> = GUIDE.iter().filter_map(|(spec, want, what)| reaches(spec, want).err().map(|e| format!("{e}   ({what})"))).collect();
    assert!(failures.is_empty(), "{} of {} shortcuts miss:\n{}", failures.len(), GUIDE.len(), failures.join("\n"));
}
