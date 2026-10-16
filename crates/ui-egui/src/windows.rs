use crate::SoundApp;
use crate::theme::{Tokens, bold, mono, rgb};
use egui::{Color32, Sense, vec2};
use openpt_model::{AutomationMode, ClipContent};
use openpt_playback::AudioConfig;
use openpt_playback::devices::{AudioSystem, DeviceInfo};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    audio_health(app, ctx);
    automation(app, ctx);
    color_palette(app, ctx);
    disk_usage(app, ctx);
    system_usage(app, ctx);
    task_manager(app, ctx);
    metadata(app, ctx);
    event_list(app, ctx);
    midi_keyboard(app, ctx);
    workspace(app, ctx);
    configurations(app, ctx);
    playback_engine(app, ctx);
    io_setup(app, ctx);
    shortcuts_window(app, ctx);
    clip_effects(app, ctx);
}

fn audio_health(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_audio_health;
    win(ctx, &mut open, "Session Audio Health", vec2(620.0, 360.0), |ui| {
        let refresh = ui.button("Refresh Report").clicked();
        if app.audio_health_report.is_none() || refresh {
            app.audio_health_report = Some(app.engine.execute("session.audio_health", &json!({})).map_err(|error| error.to_string()));
        }
        let Some(result) = &app.audio_health_report else { return };
        match result {
            Ok(report) => {
                ui.label(format!("{} audio clips checked · {} issues", report["audio_clips"], report["issue_count"]));
                ui.label(match report["active_audio_end_seconds"].as_f64() {
                    Some(seconds) => format!("Active audio ends at {seconds:.2} seconds"),
                    None => "Active audio end time is unknown (invalid session sample rate)".into(),
                });
                ui.label("Includes alternate takes and muted clips. Refresh after editing.");
                ui.separator();
                if report["healthy"] == true {
                    ui.label("All audio clips have loaded media within source bounds.");
                }
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    if let Some(issues) = report["issues"].as_array() {
                        for issue in issues {
                            ui.group(|ui| {
                                let text = |key: &str| issue[key].as_str().unwrap_or("");
                                ui.label(format!(
                                    "{} / {} / {}{}",
                                    text("track"),
                                    text("playlist"),
                                    text("clip"),
                                    if issue["active"] == true { "" } else { " (alternate playlist)" }
                                ));
                                ui.label(text("message"));
                                if let Some(path) = issue["path"].as_str() {
                                    ui.label(path);
                                }
                            });
                        }
                    }
                });
                ui.separator();
                ui.label("Checks loaded media only, not files on disk, signal levels, plugins or routing.");
            }
            Err(error) => {
                ui.label(error.to_string());
            }
        }
    });
    app.ui.show_audio_health = open;
    if !open {
        app.audio_health_report = None;
    }
}

fn win(ctx: &egui::Context, open: &mut bool, title: &str, size: egui::Vec2, body: impl FnOnce(&mut egui::Ui)) {
    if !*open {
        return;
    }
    egui::Window::new(title).open(open).default_size(size).show(ctx, body);
}

fn automation(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_automation;
    win(ctx, &mut open, "Automation", vec2(260.0, 260.0), |ui| {
        ui.label("Write automation for the selected tracks:");
        let tracks = app.engine.session().edit.selected_tracks.clone();
        ui.horizontal_wrapped(|ui| {
            for m in [
                AutomationMode::Off,
                AutomationMode::Read,
                AutomationMode::Touch,
                AutomationMode::Latch,
                AutomationMode::TouchLatch,
                AutomationMode::Write,
                AutomationMode::Trim,
            ] {
                if ui.button(m.label()).clicked() {
                    let ids: Vec<u64> = tracks.iter().map(|t| t.0).collect();
                    let _ = app.run("mix.automation_mode", json!({"tracks": ids, "mode": m.label()}));
                }
            }
        });
        ui.separator();
        ui.label("Enabled parameters");
        for f in ["volume", "pan", "mute", "send level", "send pan", "send mute", "plugin"] {
            let id = format!("automation.enable.{}", f.replace(' ', "_"));
            let mut on = !app.engine.session().edit.flag(&format!("{id}.off"));
            if crate::widgets::checkbox(ui, &mut on, f).changed() {
                app.engine.session_mut().edit.set_flag(&format!("{id}.off"), !on);
            }
        }
        ui.separator();
        if ui.button("Write to Current").clicked() {
            let _ = app.run("automation.write_to_current", json!({}));
        }
        if ui.button("Thin All").clicked() {
            let _ = app.run("automation.thin_all", json!({}));
        }
    });
    app.ui.show_automation = open;
}

#[derive(Clone, Copy, PartialEq)]
enum PaletteTarget {
    Tracks,
    Clips,
    Groups,
    Markers,
}

fn color_palette(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_color_palette;
    let id = egui::Id::new("color_palette_state");
    let (mut target, mut sat, mut val) = ctx.data(|d| d.get_temp::<(u8, f32, f32)>(id)).map_or((PaletteTarget::Tracks, 0.65, 0.85), |(t, s, v)| {
        ([PaletteTarget::Tracks, PaletteTarget::Clips, PaletteTarget::Groups, PaletteTarget::Markers][usize::from(t).min(3)], s, v)
    });
    win(ctx, &mut open, "Color Palette", vec2(560.0, 230.0), |ui| {
        ui.horizontal(|ui| {
            for (t, label) in [(PaletteTarget::Tracks, "Tracks"), (PaletteTarget::Clips, "Clips"), (PaletteTarget::Groups, "Groups"), (PaletteTarget::Markers, "Markers")] {
                ui.selectable_value(&mut target, t, label);
            }
            ui.separator();
            if ui.button("None").on_hover_text("Back to the default color").clicked() {
                apply_color(app, target, None);
            }
        });
        let mut picked = None;
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        for shade in 0..3 {
            ui.horizontal(|ui| {
                for h in 0..24 {
                    let v = val * [1.0, 0.75, 0.5][shade];
                    let c = egui::ecolor::Hsva::new(h as f32 / 24.0, sat, v, 1.0).to_srgb();
                    let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                    ui.painter().rect_filled(r, 2.0, rgb([c[0], c[1], c[2]]));
                    if resp.hovered() {
                        ui.painter().rect_stroke(r, 2.0, egui::Stroke::new(1.5, egui::Color32::WHITE), egui::StrokeKind::Inside);
                    }
                    if resp.clicked() {
                        picked = Some([c[0], c[1], c[2]]);
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            for c in openpt_model::TRACK_COLORS {
                let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                ui.painter().rect_filled(r, 2.0, rgb(c));
                if resp.clicked() {
                    picked = Some(c);
                }
            }
        });
        if let Some(c) = picked {
            apply_color(app, target, Some(c));
        }
        ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
        ui.add(egui::Slider::new(&mut sat, 0.15..=1.0).show_value(false).text("Saturation"));
        ui.add(egui::Slider::new(&mut val, 0.3..=1.0).show_value(false).text("Brightness"));
        ui.add(egui::Slider::new(&mut app.ui.track_tint, 0.0..=1.0).show_value(false).text("Track tint in the Edit window"));
    });
    ctx.data_mut(|d| d.insert_temp(id, (target as u8, sat, val)));
    app.ui.show_color_palette = open;
}

fn apply_color(app: &mut SoundApp, target: PaletteTarget, c: Option<[u8; 3]>) {
    let s = app.engine.session();
    match target {
        PaletteTarget::Tracks => {
            let tracks: Vec<(u64, usize)> =
                s.tracks.iter().enumerate().filter(|(_, t)| s.edit.selected_tracks.contains(&t.id)).map(|(i, t)| (t.id.0, i)).collect();
            for (t, i) in tracks {
                let color = c.unwrap_or(openpt_model::TRACK_COLORS[i % openpt_model::TRACK_COLORS.len()]);
                let _ = app.run("track.color", json!({"tracks": [t], "color": color}));
            }
        }
        PaletteTarget::Clips => {
            let _ = app.run("clip.color", json!({"color": c}));
        }
        PaletteTarget::Groups => {
            let Some(c) = c else { return };
            let groups: Vec<u64> = s.groups.iter().filter(|g| g.members.iter().any(|m| s.edit.selected_tracks.contains(m))).map(|g| g.id.0).collect();
            for g in groups {
                let _ = app.run("track.group_color", json!({"group": g, "color": c}));
            }
        }
        PaletteTarget::Markers => {
            let sel = s.edit.selection;
            let ids: Vec<u64> = s
                .markers
                .iter()
                .filter(|m| if sel.end > sel.start { m.start >= sel.start && m.start < sel.end } else { m.start == sel.start })
                .map(|m| m.id.0)
                .collect();
            for m in ids {
                let _ = app.run("markers.edit", json!({"id": m, "color": c}));
            }
        }
    }
}

fn disk_usage(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_disk_usage;
    win(ctx, &mut open, "Disk Usage", vec2(360.0, 160.0), |ui| {
        let s = app.engine.session();
        let bytes: u64 = s.sources.iter().map(|x| x.frames * u64::from(x.channels) * 4).sum();
        ui.label(format!("Session media: {} files, {:.1} MB in memory (32-bit float)", s.sources.len(), bytes as f64 / 1_048_576.0));
        let per_min = f64::from(s.sample_rate.hz()) * 60.0 * 3.0 / 1_048_576.0;
        ui.label(format!("Recording at 24-bit uses {per_min:.1} MB per mono track-minute"));
        if let Some(p) = &app.engine.path {
            ui.label(format!("Session file: {p}"));
        }
    });
    app.ui.show_disk_usage = open;
}

fn system_usage(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_system_usage;
    win(ctx, &mut open, "System Usage", vec2(320.0, 140.0), |ui| {
        ui.label(format!("UI frame time: {:.1} ms", app.frame_ms));
        ui.label(format!("Audio: {}", app.player.as_ref().map_or("no engine".to_string(), |p| format!("{} @ {} Hz", p.device_name, p.device_rate))));
        let plugins: usize = app.engine.session().tracks.iter().map(|t| t.mixer.inserts.iter().flatten().count()).sum();
        ui.label(format!("Active plugin instances: {plugins}"));
        ui.label(format!("Tracks: {}", app.engine.session().tracks.len()));
        ui.label(format!("Undo steps: {}", app.engine.undo_history().len()));
    });
    app.ui.show_system_usage = open;
}

fn task_manager(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_task_manager;
    win(ctx, &mut open, "Task Manager", vec2(320.0, 120.0), |ui| {
        match &app.bounce {
            Some(b) => {
                ui.label(format!("Bouncing {} ({:.0}%)", b.path().display(), b.progress() * 100.0));
                ui.add(egui::ProgressBar::new(b.progress()));
            }
            None => {
                ui.label("No background tasks are running.");
            }
        }
        ui.label(
            egui::RichText::new("Renders and AudioSuite processes run to completion before returning.").small().color(Tokens::current().text_dim),
        );
    });
    app.ui.show_task_manager = open;
}

fn metadata(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_metadata;
    win(ctx, &mut open, "Metadata Inspector", vec2(380.0, 260.0), |ui| {
        let s = app.engine.session();
        ui.label(egui::RichText::new(&s.name).strong());
        egui::Grid::new("meta").num_columns(2).striped(true).show(ui, |ui| {
            for (k, v) in [
                ("Sample rate", format!("{} Hz", s.sample_rate.hz())),
                ("Bit depth", format!("{:?}", s.bit_depth)),
                ("Timecode rate", s.frame_rate.label().to_string()),
                ("Tracks", s.tracks.len().to_string()),
                ("Clips", s.tracks.iter().map(|t| t.clips().len()).sum::<usize>().to_string()),
                ("Audio files", s.sources.len().to_string()),
                ("Markers", s.markers.len().to_string()),
                ("Length", s.format_position(s.content_end(), openpt_time::TimeFormat::MinSecs)),
            ] {
                ui.label(k);
                ui.label(v);
                ui.end_row();
            }
        });
        ui.separator();
        let mut c = s.comments.clone();
        ui.label("Session comments");
        if ui.text_edit_multiline(&mut c).changed() {
            app.engine.session_mut().comments = c;
        }
    });
    app.ui.show_metadata = open;
}

fn event_list(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_event_list;
    win(ctx, &mut open, "MIDI Event List", vec2(420.0, 360.0), |ui| {
        let Some(cid) = crate::midi_editor::target_clip(app) else {
            ui.label("Select a MIDI clip.");
            return;
        };
        let notes = match app.engine.session().find_clip(cid).map(|(_, c)| c.content.clone()) {
            Some(ClipContent::Midi { sequence }) => sequence.notes,
            _ => Vec::new(),
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("evl").num_columns(5).striped(true).show(ui, |ui| {
                for h in ["Start (ticks)", "Note", "Vel", "Length", ""] {
                    ui.label(egui::RichText::new(h).strong());
                }
                ui.end_row();
                for (i, n) in notes.iter().enumerate() {
                    ui.label(egui::RichText::new(n.start.to_string()).font(mono(11.0)));
                    ui.label(openpt_midi::note_name(n.pitch));
                    let mut v = i64::from(n.velocity);
                    if ui.add(egui::DragValue::new(&mut v).range(1..=127)).changed() {
                        let _ = app.engine.execute("midi.note_edit", &json!({"clip": cid.0, "index": i, "velocity": v}));
                    }
                    ui.label(n.length.to_string());
                    if ui.small_button("✕").clicked() {
                        let _ = app.run("midi.note_delete", json!({"clip": cid.0, "indices": [i]}));
                    }
                    ui.end_row();
                }
            });
        });
    });
    app.ui.show_event_list = open;
}

fn midi_keyboard(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_midi_keyboard;
    win(ctx, &mut open, "MIDI Keyboard", vec2(640.0, 110.0), |ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(400.0), 80.0), Sense::click_and_drag());
        let whites = 28;
        let w = r.width() / whites as f32;
        let base = 48u8;
        let pos = resp.interact_pointer_pos();

        let mut hovered_key: Option<u8> = None;
        let mut bi = 0;
        for p in base..base + 48 {
            if matches!(p % 12, 1 | 3 | 6 | 8 | 10) {
                let kr = egui::Rect::from_min_size(egui::pos2(r.min.x + bi as f32 * w - w * 0.3, r.min.y), vec2(w * 0.6, r.height() * 0.6));
                if pos.is_some_and(|q| kr.contains(q)) {
                    hovered_key = Some(p);
                    break;
                }
            } else {
                bi += 1;
            }
        }
        if hovered_key.is_none() {
            let mut wi2 = 0;
            for p in base..base + 48 {
                if !matches!(p % 12, 1 | 3 | 6 | 8 | 10) {
                    let kr = egui::Rect::from_min_size(egui::pos2(r.min.x + wi2 as f32 * w, r.min.y), vec2(w - 1.0, r.height()));
                    if pos.is_some_and(|q| kr.contains(q)) {
                        hovered_key = Some(p);
                        break;
                    }
                    wi2 += 1;
                }
            }
        }

        let is_down = (resp.clicked() || resp.dragged()) && ui.input(|i| i.pointer.primary_down());
        let active_key = if is_down { hovered_key } else { None };

        let kbd_id = ui.id().with("midi_kbd_held");
        let prev_key: Option<u8> = ui.data(|d| d.get_temp(kbd_id));
        if prev_key != active_key {
            if let Some(old) = prev_key
                && let Some(pl) = &app.player
            {
                pl.midi_input.push(openpt_midi::LiveMidiEvent::note_off(0, old, 0));
            }
            if let Some(new_p) = active_key {
                if let Some(pl) = &app.player {
                    pl.midi_input.push(openpt_midi::LiveMidiEvent::note_on(0, new_p, 100));
                }
                if resp.clicked()
                    && let Some(cid) = crate::midi_editor::target_clip(app)
                {
                    let s = app.engine.session();
                    let (clip_start, at) = (s.find_clip(cid).map_or(0, |(_, c)| c.start), s.edit.selection.start);
                    let rel = s.tempo.samples_to_ticks(at, s.sample_rate) - s.tempo.samples_to_ticks(clip_start, s.sample_rate);
                    let len = openpt_time::TICKS_PER_QUARTER / 2;
                    let _ = app.run("midi.note_add", json!({"clip": cid.0, "pitch": new_p, "start_ticks": rel.max(0), "length_ticks": len}));
                    let s = app.engine.session();
                    let next = s.tempo.tick_to_samples(s.tempo.samples_to_ticks(at, s.sample_rate) + len, s.sample_rate);
                    let _ = app.run("transport.locate", json!({"at": next}));
                }
            }
            ui.data_mut(|d| d.insert_temp(kbd_id, active_key));
        }

        let mut wi = 0;
        for p in base..base + 48 {
            if !matches!(p % 12, 1 | 3 | 6 | 8 | 10) {
                let kr = egui::Rect::from_min_size(egui::pos2(r.min.x + wi as f32 * w, r.min.y), vec2(w - 1.0, r.height()));
                let color = if active_key == Some(p) { Color32::from_rgb(180, 210, 255) } else { Color32::from_rgb(235, 235, 235) };
                ui.painter().rect_filled(kr, 2.0, color);
                wi += 1;
            }
        }
        wi = 0;
        for p in base..base + 48 {
            if matches!(p % 12, 1 | 3 | 6 | 8 | 10) {
                let kr = egui::Rect::from_min_size(egui::pos2(r.min.x + wi as f32 * w - w * 0.3, r.min.y), vec2(w * 0.6, r.height() * 0.6));
                let color = if active_key == Some(p) { Color32::from_rgb(60, 100, 180) } else { Color32::from_rgb(20, 20, 20) };
                ui.painter().rect_filled(kr, 2.0, color);
            } else {
                wi += 1;
            }
        }
        ui.label(egui::RichText::new("Click keys to step-enter notes into the selected MIDI clip at the insertion point.").small());
    });
    app.ui.show_midi_keyboard = open;
}

fn workspace(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_workspace;
    if !open {
        return;
    }
    egui::Window::new("Workspace").open(&mut open).default_size(vec2(460.0, 420.0)).show(ctx, |ui| {
        let dir = if app.ui.workspace_dir.is_empty() { std::env::var("HOME").unwrap_or_else(|_| ".".into()) } else { app.ui.workspace_dir.clone() };
        ui.horizontal(|ui| {
            if ui.button("⬆").on_hover_text("Parent folder").clicked()
                && let Some(p) = std::path::Path::new(&dir).parent()
            {
                app.ui.workspace_dir = p.to_string_lossy().into_owned();
            }
            ui.label(egui::RichText::new(&dir).small());
        });
        ui.separator();
        let mut entries: Vec<(String, bool)> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        if name.starts_with('.') {
                            return None;
                        }
                        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
                        let ext = std::path::Path::new(&name).extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                        (is_dir
                            || ["wav", "aif", "aiff", "flac", "mp3", "ogg", "m4a", "m4b", "caf", "mid", "midi", "openpt", "ptx", "ptxt"]
                                .contains(&ext.as_str()))
                        .then_some((name, is_dir))
                    })
                    .collect()
            })
            .unwrap_or_default();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (name, is_dir) in entries.into_iter().take(2000) {
                let path = std::path::Path::new(&dir).join(&name).to_string_lossy().into_owned();
                let label = if is_dir { format!("📁 {name}") } else { format!("🔊 {name}") };
                let r = ui.selectable_label(false, label);
                if r.double_clicked() {
                    if is_dir {
                        app.ui.workspace_dir = path;
                    } else if name.ends_with(".openpt") || name.ends_with(".ptx") || name.ends_with(".ptxt") {
                        let _ = crate::dialogs::run_or_ask(app, "session.open", json!({"path": path}));
                    } else if name.ends_with(".mid") || name.ends_with(".midi") {
                        let _ = app.run("file.import_midi", json!({"path": path}));
                    } else {
                        let at = app.engine.session().edit.selection.start;
                        crate::import_jobs::start(app, vec![path], at);
                    }
                }
            }
        });
        ui.label(egui::RichText::new("Double-click to import audio/MIDI at the insertion point or open a session.").small());
    });
    app.ui.show_workspace = open;
}

fn configurations(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_configurations;
    win(ctx, &mut open, "Window Configurations", vec2(320.0, 240.0), |ui| {
        if ui.button("New Configuration…").clicked() {
            let n = app.ui.configurations.len() + 1;
            let mut snap = app.ui.clone();
            snap.configurations.clear();
            if let Ok(v) = serde_json::to_value(&snap) {
                app.ui.configurations.push((format!("Configuration {n}"), v));
            }
        }
        ui.separator();
        let list = app.ui.configurations.clone();
        for (i, (name, v)) in list.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.button(format!("{}  {name}", i + 1)).clicked()
                    && let Ok(mut st) = serde_json::from_value::<crate::UiState>(v.clone())
                {
                    if let Err(error) = st.docking.validate().and_then(|()| st.mix_docking.validate_mix()) {
                        app.ui.status = format!("Window configuration could not be restored: {error}");
                    } else {
                        st.configurations = app.ui.configurations.clone();
                        st.theme = app.ui.theme;
                        st.show_configurations = true;
                        crate::docking::cancel_replaced_state(app, &st);
                        app.ui = st;
                    }
                }
                if ui.small_button("✕").clicked() {
                    app.ui.configurations.remove(i);
                }
            });
        }
    });
    app.ui.show_configurations = open;
}

fn audio_systems(ctx: &egui::Context, refresh: bool) -> Arc<Vec<AudioSystem>> {
    let id = egui::Id::new("audio_systems");
    if !refresh && let Some(s) = ctx.data(|d| d.get_temp::<Arc<Vec<AudioSystem>>>(id)) {
        return s;
    }
    let s = Arc::new(openpt_playback::devices::audio_systems());
    ctx.data_mut(|d| d.insert_temp(id, Arc::clone(&s)));
    s
}

fn chosen_system<'a>(systems: &'a [AudioSystem], cfg: &AudioConfig) -> Option<&'a AudioSystem> {
    cfg.system
        .as_deref()
        .and_then(|n| systems.iter().find(|s| s.name.eq_ignore_ascii_case(n)))
        .or_else(|| systems.iter().find(|s| s.is_default))
        .or(systems.first())
}

fn chosen_device<'a>(devices: &'a [DeviceInfo], name: Option<&str>) -> Option<&'a DeviceInfo> {
    name.and_then(|n| devices.iter().find(|d| d.name == n)).or_else(|| devices.iter().find(|d| d.is_default))
}

fn device_combo(ui: &mut egui::Ui, id: &str, value: &mut Option<String>, devices: &[DeviceInfo]) {
    let default = devices.iter().find(|d| d.is_default).map_or("none".to_string(), |d| d.name.clone());
    let shown = value.clone().unwrap_or_else(|| format!("Default ({default})"));
    egui::ComboBox::from_id_salt(id).width(300.0).selected_text(shown).show_ui(ui, |ui| {
        ui.selectable_value(value, None, format!("Default ({default})"));
        for d in devices {
            ui.selectable_value(value, Some(d.name.clone()), format!("{} ({} ch)", d.name, d.max_channels));
        }
    });
}

fn number_combo(ui: &mut egui::Ui, id: &str, value: &mut Option<u32>, options: &[u32], unit: &str) {
    let shown = value.map_or("Device default".to_string(), |v| format!("{v} {unit}"));
    egui::ComboBox::from_id_salt(id).width(300.0).selected_text(shown).show_ui(ui, |ui| {
        ui.selectable_value(value, None, "Device default");
        for v in options {
            ui.selectable_value(value, Some(*v), format!("{v} {unit}"));
        }
    });
}

pub fn playback_engine(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_playback_engine;
    if !open {
        return;
    }
    let draft_id = egui::Id::new("audio_draft");
    let mut draft = ctx.data(|d| d.get_temp::<AudioConfig>(draft_id)).unwrap_or_else(|| app.ui.audio.clone());
    let mut refresh = false;
    let mut apply = false;
    let systems = audio_systems(ctx, false);
    win(ctx, &mut open, "Playback Engine", vec2(460.0, 360.0), |ui| {
        let t = Tokens::current();
        egui::Grid::new("pe").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
            ui.label("Audio system");
            let def = systems.iter().find(|s| s.is_default).map_or("none".to_string(), |s| s.name.clone());
            let shown = draft.system.clone().unwrap_or_else(|| format!("Default ({def})"));
            egui::ComboBox::from_id_salt("pe-system").width(300.0).selected_text(shown).show_ui(ui, |ui| {
                let before = draft.system.clone();
                ui.selectable_value(&mut draft.system, None, format!("Default ({def})"));
                for s in systems.iter() {
                    ui.selectable_value(&mut draft.system, Some(s.name.clone()), &s.name);
                }
                if draft.system != before {
                    draft.output = None;
                    draft.input = None;
                }
            });
            ui.end_row();
            let sys = chosen_system(&systems, &draft);
            ui.label("Output device");
            device_combo(ui, "pe-out", &mut draft.output, sys.map_or(&[][..], |s| &s.outputs));
            ui.end_row();
            ui.label("Input device");
            device_combo(ui, "pe-in", &mut draft.input, sys.map_or(&[][..], |s| &s.inputs));
            ui.end_row();
            let out = sys.and_then(|s| chosen_device(&s.outputs, draft.output.as_deref()));
            ui.label("Sample rate");
            number_combo(ui, "pe-rate", &mut draft.sample_rate, out.map_or(&[][..], |d| &d.sample_rates), "Hz");
            ui.end_row();
            ui.label("Buffer size");
            let buffers = out.map(DeviceInfo::buffer_sizes).unwrap_or_else(|| openpt_playback::devices::STANDARD_BUFFERS.to_vec());
            number_combo(ui, "pe-buffer", &mut draft.buffer_size, &buffers, "samples");
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Refresh devices").clicked() {
                refresh = true;
            }
            let changed = draft != app.ui.audio;
            if ui.add_enabled(changed, egui::Button::new("Apply")).on_hover_text("Reopen the audio devices with these settings").clicked() {
                apply = true;
            }
            if changed && ui.button("Revert").clicked() {
                draft = app.ui.audio.clone();
            }
        });
        ui.separator();
        egui::Grid::new("pe-state").num_columns(2).show(ui, |ui| {
            ui.label("Output");
            ui.label(app.player.as_ref().map_or("none".to_string(), |p| {
                if p.silent {
                    "no device (silent clock)".to_string()
                } else {
                    let buf = p.device_buffer.map_or("default buffer".to_string(), |b| format!("{b} samples"));
                    format!("{} · {} Hz · {} ch · {buf}", p.device_name, p.device_rate, p.device_channels)
                }
            }));
            ui.end_row();
            ui.label("Input");
            ui.label(app.recorder.as_ref().map_or("opened on first record or monitor".to_string(), |r| {
                format!("{} · {} Hz · {} ch", r.device_name, r.sample_rate, r.channels)
            }));
            ui.end_row();
            ui.label("Session rate");
            let sr = app.engine.session().sample_rate.hz();
            let note = app
                .player
                .as_ref()
                .filter(|p| !p.silent && p.device_rate != sr)
                .map_or(String::new(), |_| " (resampled to the device rate)".to_string());
            ui.label(format!("{sr} Hz{note}"));
            ui.end_row();
        });
        ui.label(egui::RichText::new("The mix runs in 512-sample blocks; the buffer size sets the device's own latency.").small().color(t.text_dim));
        let mut dc = app.engine.session().edit.delay_compensation;
        if crate::widgets::checkbox(ui, &mut dc, "Delay compensation").changed() {
            let _ = app.run("options.delay_compensation", json!({"value": dc}));
        }
        if ui.button("Reconnect audio device").clicked() {
            app.reconnect_audio();
        }
    });
    if refresh {
        audio_systems(ctx, true);
    }
    if apply && let Err(e) = app.apply_audio_config(draft.clone()) {
        app.ui.status = e;
    }
    if open {
        ctx.data_mut(|d| d.insert_temp(draft_id, draft));
    } else {
        ctx.data_mut(|d| {
            d.remove_temp::<AudioConfig>(draft_id);
            d.remove_temp::<Arc<Vec<AudioSystem>>>(egui::Id::new("audio_systems"));
        });
    }
    app.ui.show_playback_engine = open;
}

pub fn audio_device_command(app: &mut SoundApp, p: &serde_json::Value) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    let mut changed = false;
    let mut cfg = app.ui.audio.clone();
    for (key, slot) in [("system", &mut cfg.system), ("output", &mut cfg.output), ("input", &mut cfg.input)] {
        if let Some(v) = p.get(key) {
            *slot = v.as_str().map(str::to_string).filter(|s| !s.is_empty());
            changed = true;
        }
    }
    for (key, slot, range) in [("sample_rate", &mut cfg.sample_rate, 8_000..=768_000u64), ("buffer_size", &mut cfg.buffer_size, 16..=8192u64)] {
        if let Some(v) = p.get(key) {
            *slot = match v {
                Value::Null => None,
                v => Some(v.as_u64().filter(|n| range.contains(n)).ok_or_else(|| format!("`{key}` must be a number in {range:?} or null"))? as u32),
            };
            changed = true;
        }
    }
    if changed {
        app.apply_audio_config(cfg)?;
    }
    let device = app.player.as_ref().map(|pl| {
        json!({"name": pl.device_name, "sample_rate": pl.device_rate, "channels": pl.device_channels, "buffer_size": pl.device_buffer, "silent": pl.silent})
    });
    let mut out = json!({"config": app.ui.audio, "device": device});
    if p.get("list").and_then(Value::as_bool).unwrap_or(false) {
        out["systems"] = json!(openpt_playback::devices::audio_systems());
    }
    Ok(out)
}
pub fn io_setup(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_io_setup;
    if !open {
        return;
    }
    let page_id = egui::Id::new("io_setup_page");
    let mut page: u8 = ctx.data(|d| d.get_temp(page_id)).unwrap_or(0);
    let systems = audio_systems(ctx, false);
    let system = chosen_system(&systems, &app.ui.audio);
    let device = |inputs: bool| {
        system.and_then(|s| {
            let list = if inputs { &s.inputs } else { &s.outputs };
            chosen_device(list, if inputs { app.ui.audio.input.as_deref() } else { app.ui.audio.output.as_deref() })
        })
    };
    let (in_dev, out_dev) = (device(true).cloned(), device(false).cloned());
    let mut runs: Vec<Value> = Vec::new();
    egui::Window::new("I/O Setup").open(&mut open).default_size(vec2(860.0, 440.0)).show(ctx, |ui| {
        let t = Tokens::current();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, label) in ["Input", "Output", "Bus"].into_iter().enumerate() {
                let on = page == i as u8;
                let text = egui::RichText::new(label).font(bold(12.5)).color(if on { t.text } else { t.text_dim });
                if ui.add(egui::Button::new(text).fill(if on { t.group_bg } else { t.group_header }).min_size(vec2(96.0, 24.0))).clicked() {
                    page = i as u8;
                }
            }
        });
        ui.separator();
        let s = app.engine.session();
        match page {
            0 | 1 => {
                let inputs = page == 0;
                let dev = if inputs { in_dev.as_ref() } else { out_dev.as_ref() };
                let paths = if inputs { s.inputs.clone() } else { s.outputs.clone() };
                let used = paths.iter().map(|p| usize::from(p.first_channel) + p.format.channels()).max().unwrap_or(0);
                let n = dev.map_or(8, |d| usize::from(d.max_channels)).max(used).clamp(1, 64);
                let interface = dev.map_or_else(
                    || "No audio device".to_string(),
                    |d| format!("{} — {} {}", d.name, d.max_channels, if inputs { "inputs" } else { "outputs" }),
                );
                ui.label(egui::RichText::new(interface).color(t.text_dim));
                io_grid(ui, &paths, n, dev.map_or(n, |d| usize::from(d.max_channels)), inputs, &mut runs);
                ui.horizontal(|ui| {
                    if let Some((count, fmt)) = new_paths(ui, if inputs { "io_new_in" } else { "io_new_out" }, if inputs { "input" } else { "output" }) {
                        let prefix = if inputs { "In" } else { "Out" };
                        let mut taken: Vec<String> = paths.iter().map(|p| p.name.to_ascii_lowercase()).collect();
                        let mut first = used;
                        for _ in 0..count {
                            let Some(name) = (1..1000).map(|k| format!("{prefix} {k}")).find(|c| !taken.contains(&c.to_ascii_lowercase())) else { break };
                            taken.push(name.to_ascii_lowercase());
                            let at = first.min(n.saturating_sub(fmt.channels()));
                            runs.push(json!({"action": if inputs { "create_input" } else { "create_output" }, "name": name, "first_channel": at, "format": fmt.label()}));
                            first += fmt.channels();
                        }
                    }
                    if !inputs {
                        ui.separator();
                        ui.label("Main output:");
                        let main = s.main_format();
                        egui::ComboBox::from_id_salt("main_output_format").selected_text(main.label()).show_ui(ui, |ui| {
                            for f in openpt_model::ChannelFormat::ALL.into_iter().filter(|f| (2..=16).contains(&f.channels())) {
                                if ui.selectable_label(f == main, f.label()).clicked() && f != main {
                                    runs.push(json!({"main_format": f.label()}));
                                }
                            }
                        });
                    }
                });
            }
            _ => {
                bus_page(ui, s, &mut runs);
            }
        }
    });
    for p in runs {
        let r = if let Some(f) = p.get("main_format").and_then(Value::as_str) {
            app.run("setup.main_format", json!({"format": f}))
        } else {
            app.run("setup.io", p)
        };
        if let Err(e) = r {
            app.ui.status = e;
        }
    }
    ctx.data_mut(|d| d.insert_temp(page_id, page));
    app.ui.show_io_setup = open;
}

fn name_field(ui: &mut egui::Ui, key: impl std::hash::Hash + std::fmt::Debug, name: &str) -> Option<String> {
    let id = egui::Id::new(key);
    let mut edit: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| name.to_string());
    let r = ui.add(egui::TextEdit::singleline(&mut edit).desired_width(150.0));
    let done = r.lost_focus() && !edit.trim().is_empty() && edit.trim() != name;
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, edit.clone()));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    done.then(|| edit.trim().to_string())
}

fn io_grid(ui: &mut egui::Ui, paths: &[openpt_model::OutputPath], n: usize, real: usize, inputs: bool, runs: &mut Vec<Value>) {
    let t = Tokens::current();
    let (set, rename, delete) = if inputs { ("set_input", "rename_input", "delete_input") } else { ("set_output", "rename_output", "delete_output") };
    let cell = 22.0;
    egui::ScrollArea::both().max_height(320.0).show(ui, |ui| {
        egui::Grid::new(("io_grid", inputs)).num_columns(4).spacing(vec2(8.0, 3.0)).show(ui, |ui| {
            ui.label(egui::RichText::new("Name").strong());
            ui.label(egui::RichText::new("Format").strong());
            let (r, _) = ui.allocate_exact_size(vec2(cell * n as f32, 18.0), egui::Sense::hover());
            for c in 0..n {
                let x = r.min.x + cell * (c as f32 + 0.5);
                ui.painter().text(
                    egui::pos2(x, r.center().y),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", c + 1),
                    bold(11.0),
                    if c < real { t.text } else { t.text_dim.gamma_multiply(0.5) },
                );
            }
            ui.end_row();
            for p in paths {
                if let Some(to) = name_field(ui, ("io_name", inputs, &p.name), &p.name) {
                    runs.push(json!({"action": rename, "name": p.name, "new_name": to}));
                }
                egui::ComboBox::from_id_salt(("io_fmt", inputs, &p.name)).width(80.0).selected_text(p.format.label()).show_ui(ui, |ui| {
                    for f in openpt_model::ChannelFormat::ALL.into_iter().filter(|f| f.channels() <= 8) {
                        if ui.selectable_label(f == p.format, f.label()).clicked() && f != p.format {
                            runs.push(json!({"action": set, "name": p.name, "format": f.label()}));
                        }
                    }
                });
                let (r, resp) = ui.allocate_exact_size(vec2(cell * n as f32, 18.0), egui::Sense::click());
                let first = usize::from(p.first_channel);
                let width = p.format.channels();
                for c in 0..n {
                    let cr = egui::Rect::from_min_size(egui::pos2(r.min.x + cell * c as f32, r.min.y), vec2(cell - 2.0, r.height()));
                    let lit = (first..first + width).contains(&c);
                    ui.painter().rect(
                        cr,
                        1.5,
                        if lit {
                            t.accent
                        } else if c < real {
                            t.field_bg
                        } else {
                            t.group_header
                        },
                        egui::Stroke::new(1.0, t.border_light),
                        egui::StrokeKind::Inside,
                    );
                    if lit && width > 1 {
                        ui.painter().text(
                            cr.center(),
                            egui::Align2::CENTER_CENTER,
                            p.format.channel_label(c - first),
                            bold(9.5),
                            egui::Color32::WHITE,
                        );
                    }
                }
                if resp.clicked()
                    && let Some(pos) = resp.interact_pointer_pos()
                {
                    let c = (((pos.x - r.min.x) / cell).floor().max(0.0) as usize).min(n.saturating_sub(width.max(1)));
                    if c != first {
                        runs.push(json!({"action": set, "name": p.name, "first_channel": c}));
                    }
                }
                if ui.small_button("Delete").clicked() {
                    runs.push(json!({"action": delete, "name": p.name}));
                }
                ui.end_row();
            }
        });
    });
}

pub fn shortcuts_window(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_shortcuts;
    let mac = ctx.os().is_mac();
    win(ctx, &mut open, "Keyboard Shortcuts", vec2(520.0, 480.0), |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("sc").num_columns(3).striped(true).show(ui, |ui| {
                let engine = openpt_engine::command_specs().iter().map(|c| (c.shortcut, c.label, c.menu.join(" › ")));
                let windows = crate::menus::UI_COMMANDS.iter().map(|c| (c.3, c.1, c.2.replace(" > ", " › ")));
                for (shortcut, label, menu) in engine.chain(windows) {
                    let Some(shortcut) = shortcut else { continue };
                    ui.label(egui::RichText::new(crate::shortcuts::shortcut_label(shortcut, mac)).font(mono(11.0)));
                    ui.label(label);
                    ui.label(egui::RichText::new(menu).small());
                    ui.end_row();
                }
            });
        });
    });
    app.ui.show_shortcuts = open;
}

pub fn clip_effects(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_clip_effects;
    win(ctx, &mut open, "Clip Effects", vec2(560.0, 420.0), |ui| {
        let clip = app.engine.session().edit.selected_clips.first().copied().or_else(|| {
            let s = app.engine.session();
            s.tracks
                .iter()
                .filter(|t| s.edit.selected_tracks.contains(&t.id))
                .flat_map(|t| t.clips().iter())
                .find(|c| c.range().contains(s.edit.selection.start) && c.is_audio())
                .map(|c| c.id)
        });
        let Some(cid) = clip else {
            ui.label("Select an audio clip.");
            return;
        };
        let name = app.engine.session().find_clip(cid).map(|(_, c)| c.name.clone()).unwrap_or_default();
        let bypass = app.engine.session().edit.flag(&format!("clip_fx.bypass.{}", cid.0));
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&name).strong());
            if ui.selectable_label(bypass, "Bypass").clicked() {
                let _ = app.run("clip.effects_bypass", json!({"clips": [cid.0]}));
            }
            if ui.button("Render").clicked() {
                let _ = app.run("clip.effects_render", json!({"clips": [cid.0]}));
            }
            if ui.button("Clear").clicked() {
                let _ = app.run("edit.clear_clip_effects", json!({"clips": [cid.0]}));
            }
        });
        let get = |app: &SoundApp, k: &str, d: f32| app.engine.session().edit.values.get(&format!("clip_fx.{}.{k}", cid.0)).map_or(d, |v| *v as f32);
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.columns(2, |cols| {
                for (col, (title, plugin, prefix)) in cols.iter_mut().zip([("EQ", "eq_7band", "eq."), ("Dynamics", "compressor", "comp.")]) {
                    col.label(egui::RichText::new(title).strong());
                    let Some(info) = openpt_dsp::plugin_info(plugin) else { continue };
                    for p in info.params {
                        let key = format!("{prefix}{}", p.id);
                        let mut v = get(app, &key, p.default);
                        let before = v;
                        col.add(egui::Slider::new(&mut v, p.min..=p.max).text(p.name));
                        if (v - before).abs() > f32::EPSILON {
                            let mut m = serde_json::Map::new();
                            m.insert(key.clone(), json!(v));
                            let _ = app.engine.execute_merged(
                                "clip.effects_set",
                                &json!({"clips": [cid.0], "params": m}),
                                &format!("cfx:{}:{key}", cid.0),
                            );
                        }
                    }
                }
            });
            let mut g = get(app, "gain", 0.0);
            let before = g;
            ui.add(egui::Slider::new(&mut g, -36.0..=24.0).text("Clip effects gain (dB)"));
            if (g - before).abs() > f32::EPSILON {
                let _ =
                    app.engine.execute_merged("clip.effects_set", &json!({"clips": [cid.0], "params": {"gain": g}}), &format!("cfx:{}:gain", cid.0));
            }
        });
    });
    app.ui.show_clip_effects = open;
}

fn path_formats() -> impl Iterator<Item = openpt_model::ChannelFormat> {
    openpt_model::ChannelFormat::ALL.into_iter().filter(|f| f.channels() <= 10)
}

fn new_paths(ui: &mut egui::Ui, id: &str, what: &str) -> Option<(usize, openpt_model::ChannelFormat)> {
    let key = egui::Id::new((id, "new_paths"));
    let (mut count, mut fmt): (usize, openpt_model::ChannelFormat) = ui
        .data(|d| d.get_temp(key))
        .unwrap_or((1, if what == "bus" { openpt_model::ChannelFormat::Stereo } else { openpt_model::ChannelFormat::Mono }));
    let mut go = false;
    egui::Frame::new().fill(Tokens::current().group_header).inner_margin(egui::Margin::symmetric(10, 6)).corner_radius(3).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Create");
            ui.add(egui::DragValue::new(&mut count).range(1..=64).speed(0.1));
            ui.label("new");
            egui::ComboBox::from_id_salt((id, "fmt")).width(90.0).selected_text(fmt.label()).show_ui(ui, |ui| {
                for f in path_formats() {
                    ui.selectable_value(&mut fmt, f, f.label());
                }
            });
            ui.label(if count > 1 { format!("{what}{}", if what == "bus" { "ses" } else { "s" }) } else { what.to_string() });
            ui.add_space(8.0);
            go = ui.add(egui::Button::new(egui::RichText::new("Create").strong()).min_size(vec2(70.0, 0.0))).clicked();
        });
    });
    ui.data_mut(|d| d.insert_temp(key, (count, fmt)));
    go.then_some((count, fmt))
}

fn bus_page(ui: &mut egui::Ui, s: &openpt_model::Session, runs: &mut Vec<Value>) {
    const COLS: [f32; 4] = [170.0, 112.0, 70.0, 90.0];
    let t = Tokens::current();
    let uses = |b: openpt_model::BusId| {
        let r = openpt_model::Route::Bus(b);
        s.tracks.iter().filter(|x| x.mixer.input == r || x.mixer.output == r || x.mixer.sends.iter().flatten().any(|sn| sn.target == r)).count()
    };
    let rows: Vec<(String, openpt_model::ChannelFormat, usize)> = s.busses.iter().map(|b| (b.name.clone(), b.format, uses(b.id))).collect();
    let cell = |ui: &mut egui::Ui, w: f32, f: &mut dyn FnMut(&mut egui::Ui)| {
        ui.allocate_ui_with_layout(vec2(w, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(w);
            f(ui);
        });
    };
    egui::Frame::new().fill(t.group_header).inner_margin(egui::Margin::symmetric(8, 3)).corner_radius(2).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            for (text, w) in ["Name", "Format", "Channels", "Used by"].into_iter().zip(COLS) {
                cell(ui, w, &mut |ui| {
                    ui.label(egui::RichText::new(text).font(bold(11.5)).color(t.text_dim));
                });
            }
        });
    });
    egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if rows.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("No busses yet.").color(t.text_dim));
        }
        for (i, (name, fmt, used)) in rows.into_iter().enumerate() {
            let fill = if i % 2 == 0 { t.field_bg } else { t.group_bg };
            egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    cell(ui, COLS[0], &mut |ui| {
                        if let Some(to) = name_field(ui, ("bus_name", &name), &name) {
                            runs.push(json!({"action": "rename_bus", "name": name, "new_name": to}));
                        }
                    });
                    cell(ui, COLS[1], &mut |ui| {
                        egui::ComboBox::from_id_salt(("bus_fmt", &name)).width(100.0).selected_text(fmt.label()).show_ui(ui, |ui| {
                            for f in path_formats() {
                                if ui.selectable_label(f == fmt, f.label()).clicked() && f != fmt {
                                    runs.push(json!({"action": "set_bus", "name": name, "format": f.label()}));
                                }
                            }
                        });
                    });
                    cell(ui, COLS[2], &mut |ui| {
                        ui.label(fmt.channels().to_string());
                    });
                    cell(ui, COLS[3], &mut |ui| {
                        let label = match used {
                            0 => "unused".to_string(),
                            1 => "1 track".to_string(),
                            n => format!("{n} tracks"),
                        };
                        ui.label(egui::RichText::new(label).color(t.text_dim));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Delete").on_hover_text("Delete this bus (tracks using it lose the route)").clicked() {
                            runs.push(json!({"action": "delete_bus", "name": name}));
                        }
                    });
                });
            });
        }
    });
    ui.add_space(8.0);
    if let Some((count, fmt)) = new_paths(ui, "io_new_bus", "bus") {
        runs.push(json!({"action": "create_busses", "name": "Bus", "count": count, "format": fmt.label()}));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod audio_health_tests {
    use super::*;

    #[test]
    fn window_command_renders_report_and_close_clears_snapshot() {
        let mut app = SoundApp::new(openpt_engine::Engine::default(), None, crate::Services::default());
        app.run("window.audio_health", json!({"value": true})).unwrap();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| audio_health(&mut app, ui.ctx()));
        output.textures_delta.clear();
        assert_eq!(app.audio_health_report.as_ref().unwrap().as_ref().unwrap()["healthy"], true);
        assert!(!app.engine.is_dirty());
        app.run("window.hide_floating", json!({})).unwrap();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| audio_health(&mut app, ui.ctx()));
        output.textures_delta.clear();
        assert!(!app.ui.show_audio_health);
        assert!(app.audio_health_report.is_none());
        app.run("window.audio_health", json!({"value": true})).unwrap();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| audio_health(&mut app, ui.ctx()));
        output.textures_delta.clear();
        assert!(app.audio_health_report.is_some());
    }
}
