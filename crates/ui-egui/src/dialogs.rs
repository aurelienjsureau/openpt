use crate::SoundApp;
use crate::theme::{Tokens, bold};
use egui::{Align2, Color32, vec2};
use openpt_engine::Engine;
use openpt_model::{ClipId, Session, SourceId, TrackId};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct NewTrackRow {
    pub count: u32,
    pub format: String,
    pub kind: String,
    pub timebase: String,
    pub name: String,
}

impl Default for NewTrackRow {
    fn default() -> Self {
        NewTrackRow { count: 1, format: "Mono".into(), kind: "audio".into(), timebase: "samples".into(), name: "Audio".into() }
    }
}

#[derive(Debug, Clone)]
pub enum Dialog {
    NewTracks {
        rows: Vec<NewTrackRow>,
    },
    Bounce {
        path: String,
        format: String,
        bit_depth: String,
        normalize: bool,
        dither: bool,
        realtime: bool,
    },
    RenameTrack {
        id: TrackId,
        name: String,
    },
    RenameClip {
        id: ClipId,
        name: String,
    },
    RenameMarker {
        id: openpt_model::MarkerId,
        name: String,
    },
    ClearClips {
        clips: Vec<ClipId>,
        sources: Vec<SourceId>,
        message: String,
    },
    PathPrompt {
        cmd: String,
        title: String,
        path: String,
        key: String,
    },
    Number {
        cmd: String,
        title: String,
        key: String,
        value: f64,
        suffix: String,
    },
    TempoChange {
        loc: String,
        bpm: f64,
        from_tick: Option<i64>,
        snap_bar: bool,
    },
    Spot(crate::spot::Spot),
    Fades {
        shape: String,
        batch: Option<BatchFades>,
    },
    ImportSessionData(Box<ImportSessionData>),
    LinkedChanged {
        path: String,
        tracks: Vec<String>,
    },
    NewFolder {
        name: String,
        routing: bool,
        route: bool,
        tracks: Vec<u64>,
    },
    StripSilence {
        threshold: f64,
        min_ms: f64,
        pre_ms: f64,
        post_ms: f64,
    },
    Group {
        name: String,
        edit: bool,
        mix: bool,
        members: Vec<(u64, String, bool)>,
        modify: Option<u64>,
    },
    Conform {
        bpm: String,
        bar: i64,
        grid: String,
        refs: Vec<(u64, String, bool)>,
        all_tracks: bool,
    },
    Session {
        frame_rate: String,
        bit_depth: String,
    },
    ScoreSetup {
        title: String,
        composer: String,
        bars_per_system: u32,
        show_track_names: bool,
    },
    MidiInputDevices {
        devices: Vec<(String, bool)>,
    },
    SaveChanges {
        then: (String, Value),
    },
}

#[derive(Default)]
pub struct Dialogs {
    pub open: Option<Dialog>,
    after_save: Option<(String, Value)>,
}

impl Dialogs {
    pub fn open_name(&self) -> Option<&'static str> {
        self.open.as_ref().map(|d| match d {
            Dialog::NewTracks { .. } => "new_tracks",
            Dialog::Bounce { .. } => "bounce",
            Dialog::RenameTrack { .. } => "rename_track",
            Dialog::RenameClip { .. } => "rename_clip",
            Dialog::RenameMarker { .. } => "rename_marker",
            Dialog::ClearClips { .. } => "clear_clips",
            Dialog::PathPrompt { .. } => "path",
            Dialog::Number { .. } => "number",
            Dialog::TempoChange { .. } => "tempo_change",
            Dialog::Spot(_) => "spot",
            Dialog::Fades { .. } => "fades",
            Dialog::StripSilence { .. } => "strip_silence",
            Dialog::NewFolder { .. } => "new_folder",
            Dialog::ImportSessionData(_) => "import_session_data",
            Dialog::LinkedChanged { .. } => "linked_changed",
            Dialog::Group { .. } => "group",
            Dialog::Conform { .. } => "conform",
            Dialog::Session { .. } => "session",
            Dialog::ScoreSetup { .. } => "score_setup",
            Dialog::MidiInputDevices { .. } => "midi_input_devices",
            Dialog::SaveChanges { .. } => "save_changes",
        })
    }

    pub fn open_modify_group(&mut self, s: &openpt_model::Session, group: u64) {
        let Some(g) = s.groups.iter().find(|g| g.id.0 == group) else { return };
        let members = s.tracks.iter().map(|t| (t.id.0, t.name.clone(), g.members.contains(&t.id))).collect();
        self.open = Some(Dialog::Group { name: g.name.clone(), edit: g.edit, mix: g.mix, members, modify: Some(group) });
    }

    pub fn open_import_session_data(&mut self, e: &mut Engine, path: &str) -> Result<(), String> {
        let v = e.execute("file.session_data_tracks", &json!({"path": path})).map_err(|e| e.to_string())?;
        let rows = v["tracks"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|t| ImportRow {
                        name: t["name"].as_str().unwrap_or_default().to_string(),
                        kind: t["kind"].as_str().unwrap_or_default().to_string(),
                        folder: t["folder"].as_str().map(str::to_string),
                        clips: t["clips"].as_u64().unwrap_or(0),
                        on: true,
                        dest: String::new(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.open = Some(Dialog::ImportSessionData(Box::new(ImportSessionData {
            path: path.to_string(),
            name: v["name"].as_str().unwrap_or_default().to_string(),
            rows,
            data: [true; openpt_engine::cmd::TRACK_DATA.len()],
            markers: false,
            tempo: false,
            link: false,
            has_markers: v["markers"].as_u64().unwrap_or(0) > 0,
            has_tempo: v["tempo_changes"].as_u64().unwrap_or(0) > 1,
        })));
        Ok(())
    }

    pub fn open_new_folder(&mut self, tracks: Vec<u64>) -> bool {
        if tracks.is_empty() {
            self.open = None;
            return false;
        }
        self.open = Some(Dialog::NewFolder { name: "Folder".into(), routing: true, route: true, tracks });
        true
    }

    pub fn open_rename_track(&mut self, id: TrackId, name: &str) {
        self.open = Some(Dialog::RenameTrack { id, name: name.to_string() });
    }

    pub fn open_rename_clip(&mut self, id: ClipId, name: &str) {
        self.open = Some(Dialog::RenameClip { id, name: name.to_string() });
    }

    pub fn open_rename_marker(&mut self, id: openpt_model::MarkerId, name: &str) {
        self.open = Some(Dialog::RenameMarker { id, name: name.to_string() });
    }

    pub fn open_clear(&mut self, e: &Engine, clips: Vec<ClipId>, sources: Vec<SourceId>) -> bool {
        let Some(message) = clear_message(e.session(), &clips, &sources) else { return false };
        self.open = Some(Dialog::ClearClips { clips, sources, message });
        true
    }

    pub fn open_for_command(&mut self, e: &Engine, id: &str) -> bool {
        self.after_save = None;
        let home = default_dir();
        let name = e.session().name.clone();
        self.open = Some(match id {
            "clip.clear" => {
                let ed = &e.session().edit;
                return self.open_clear(e, ed.selected_clips.clone(), ed.selected_sources.clone());
            }
            "track.new" => Dialog::NewTracks { rows: vec![NewTrackRow::default()] },
            "file.bounce_mix" => {
                let dir = e
                    .path
                    .as_deref()
                    .and_then(|p| std::path::Path::new(p).parent())
                    .filter(|d| !d.as_os_str().is_empty())
                    .map_or(home.clone(), |d| d.display().to_string());
                Dialog::Bounce {
                    path: format!("{dir}/{name} Bounce.wav"),
                    format: "wav".into(),
                    bit_depth: "24".into(),
                    normalize: false,
                    dither: false,
                    realtime: false,
                }
            }
            "file.import_audio" => Dialog::PathPrompt { cmd: id.into(), title: "Import Audio".into(), path: home, key: "path".into() },
            "file.import_midi" => Dialog::PathPrompt { cmd: id.into(), title: "Import MIDI".into(), path: home, key: "path".into() },
            "event.add_key_change" => Dialog::PathPrompt {
                cmd: id.into(),
                title: "Add Key Change (e.g. G major, E minor)".into(),
                path: "C major".into(),
                key: "key".into(),
            },
            "event.add_chord_change" => {
                Dialog::PathPrompt { cmd: id.into(), title: "Add Chord Change (e.g. Am7)".into(), path: "C".into(), key: "chord".into() }
            }
            "session.open" => Dialog::PathPrompt { cmd: id.into(), title: "Open Session".into(), path: home, key: "path".into() },
            "session.save" if e.path.is_some() => return false,
            "session.save" | "session.save_as" | "session.save_copy" => Dialog::PathPrompt {
                cmd: if id == "session.save_copy" { id } else { "session.save_as" }.into(),
                title: "Save Session As".into(),
                path: if home.is_empty() {
                    format!("{name}.{}", openpt_engine::io::BUNDLE_EXTENSION)
                } else {
                    format!("{home}/{name}/{name}.openpt")
                },
                key: "path".into(),
            },
            "session.save_template" => Dialog::PathPrompt {
                cmd: id.into(),
                title: "Save As Template".into(),
                path: crate::dashboard::template_dirs()
                    .first()
                    .map_or_else(|| format!("{name}.openpt"), |d| d.join(format!("{name}.openpt")).display().to_string()),
                key: "path".into(),
            },
            "edit.repeat" => Dialog::Number { cmd: id.into(), title: "Repeat".into(), key: "count".into(), value: 2.0, suffix: "times".into() },
            "edit.shift" => {
                Dialog::Number { cmd: id.into(), title: "Shift (seconds)".into(), key: "by_seconds".into(), value: 1.0, suffix: "s".into() }
            }
            "event.transpose" => {
                Dialog::Number { cmd: id.into(), title: "Transpose".into(), key: "semitones".into(), value: 12.0, suffix: "semitones".into() }
            }
            "event.quantize" => {
                Dialog::Number { cmd: id.into(), title: "Quantize strength".into(), key: "strength".into(), value: 100.0, suffix: "%".into() }
            }
            "edit.fades_create" => Dialog::Fades { shape: "equal power".into(), batch: BatchFades::wanted(&e.session()) },
            "edit.strip_silence" => Dialog::StripSilence { threshold: -48.0, min_ms: 50.0, pre_ms: 5.0, post_ms: 20.0 },
            "clip.warp_conform" => {
                let s = e.session();
                let refs = s
                    .tracks
                    .iter()
                    .filter(|t| t.kind == openpt_model::TrackKind::Audio)
                    .map(|t| {
                        let n = t.name.to_lowercase();
                        (t.id.0, t.name.clone(), n.contains("kick") || n.contains("snare"))
                    })
                    .collect();
                Dialog::Conform { bpm: String::new(), bar: 1, grid: "1/16".into(), refs, all_tracks: true }
            }
            "track.move_to_new_folder" => {
                let tracks = e.session().edit.selected_tracks.iter().map(|t| t.0).collect();
                return self.open_new_folder(tracks);
            }
            "track.group" => {
                let s = e.session();
                let members = s.tracks.iter().map(|t| (t.id.0, t.name.clone(), s.edit.selected_tracks.contains(&t.id))).collect();
                Dialog::Group { name: format!("Group {}", s.groups.len() + 1), edit: true, mix: true, members, modify: None }
            }
            "setup.session" => Dialog::Session { frame_rate: e.session().frame_rate.label().into(), bit_depth: "24".into() },
            "file.score_setup" => {
                let s = openpt_engine::score::ScoreSetup::from_session(e.session());
                Dialog::ScoreSetup { title: s.title, composer: s.composer, bars_per_system: s.bars_per_system, show_track_names: s.show_track_names }
            }
            "clip.rename" => {
                let target = openpt_engine::cmd::clip_ids_param(e, &json!({})).first().and_then(|id| e.session().find_clip(*id));
                let Some((_, clip)) = target else { return false };
                Dialog::RenameClip { id: clip.id, name: clip.name.clone() }
            }
            "setup.midi_input_devices" => {
                let devices = openpt_playback::midi::list_input_devices();
                let flags = &e.session().edit.flags;
                let any_flag = flags.iter().any(|f| f.starts_with("midi.input."));
                let items = devices
                    .into_iter()
                    .map(|d| {
                        let enabled = if any_flag { flags.contains(&format!("midi.input.{}", d.name.trim())) } else { true };
                        (d.name, enabled)
                    })
                    .collect();
                Dialog::MidiInputDevices { devices: items }
            }
            _ => match path_dialog(id, &home, &name) {
                Some(d) => d,
                None => return false,
            },
        });
        true
    }
}

pub fn ask_to_save(app: &mut SoundApp, id: &str, params: &Value) -> bool {
    if !matches!(id, "session.new" | "session.open" | "session.close" | "session.revert" | "app.quit")
        || openpt_engine::find_command(id).is_some_and(|c| (c.enabled)(&app.engine).is_err())
    {
        return false;
    }
    if app.engine.transport.recording {
        app.stop_play();
    }
    if !app.engine.is_dirty() {
        return false;
    }
    app.dialogs.open = Some(Dialog::SaveChanges { then: (id.into(), params.clone()) });
    true
}

pub fn run_or_ask(app: &mut SoundApp, id: &str, params: Value) -> Result<Value, String> {
    if ask_to_save(app, id, &params) {
        return Ok(Value::Null);
    }
    app.run(id, params)
}

pub fn takes_path(id: &str) -> bool {
    path_key(id).is_some()
}

fn path_key(id: &str) -> Option<&'static str> {
    let spec = openpt_engine::command_specs().iter().find(|c| c.id == id)?;
    if spec.params.starts_with("{path?") && id != "file.print_score" {
        None
    } else if spec.params.starts_with("{path") {
        Some("path")
    } else if spec.params.starts_with("{dir") {
        Some("dir")
    } else {
        None
    }
}

fn path_dialog(id: &str, home: &str, name: &str) -> Option<Dialog> {
    let spec = openpt_engine::command_specs().iter().find(|c| c.id == id)?;
    let key = path_key(id)?;
    let importing = id.contains("import") || id.contains("open");
    let ext = match id {
        "file.export_midi" => ".mid",
        "file.export_sibelius" => ".musicxml",
        "file.print_score" => ".svg",
        "file.export_clip_groups" => ".scgrp",
        "file.export_session_text" => ".txt",
        _ if id.contains("session") => ".openpt",
        _ => "",
    };
    let path = if importing || key == "dir" { format!("{home}/") } else { format!("{home}/{name}{ext}") };
    let title = format!("{} — {}", spec.menu.last().copied().unwrap_or("File"), spec.label.trim_end_matches("..."));
    Some(Dialog::PathPrompt { cmd: id.into(), title, path, key: key.into() })
}

fn clear_message(s: &Session, clips: &[ClipId], sources: &[SourceId]) -> Option<String> {
    let listed: std::collections::BTreeSet<ClipId> = clips.iter().copied().collect();
    let names: Vec<&str> = s.tracks.iter().flat_map(|t| t.clips()).filter(|c| listed.contains(&c.id)).map(|c| c.name.as_str()).collect();
    let files: Vec<_> = sources.iter().filter_map(|id| s.source(*id)).collect();
    let count = |n: usize, what: &str| if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") };
    let what = match (names.as_slice(), files.as_slice()) {
        ([], []) => return None,
        ([name], []) => format!("the clip \"{name}\""),
        ([], [f]) => format!("the file \"{}\"", f.name),
        (c, []) => count(c.len(), "clip"),
        ([], f) => count(f.len(), "file"),
        (c, f) => format!("{} and {}", count(c.len(), "clip"), count(f.len(), "file")),
    };
    let mut text = format!("Remove {what} from the session?");
    let used: Vec<(&str, usize)> =
        files.iter().map(|f| (f.name.as_str(), s.clips_using(f.id).filter(|c| !listed.contains(&c.id)).count())).filter(|(_, n)| *n > 0).collect();
    let n: usize = used.iter().map(|u| u.1).sum();
    let users = if n == 1 {
        "a clip on the timeline, which is removed too.".to_string()
    } else {
        format!("{n} clips on the timeline, which are removed too.")
    };
    match used.as_slice() {
        [] => {}
        [_] if files.len() == 1 => text += &format!(" It is used by {users}"),
        u if u.len() == files.len() => text += &format!(" They are used by {users}"),
        [(name, _)] => text += &format!(" \"{name}\" is used by {users}"),
        many => text += &format!(" {} of the files are used by {users}", many.len()),
    }
    if !files.is_empty() && files.iter().all(|f| !f.unsaved) {
        text += if files.len() == 1 { " The file stays on disk." } else { " The files stay on disk." };
    }
    Some(text)
}

pub(crate) fn default_dir() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into())
    }
    #[cfg(target_arch = "wasm32")]
    {
        String::new()
    }
}

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    audiosuite_window(app, ctx);
    crate::bounce::show(app, ctx);
    crate::bounce::show_realtime(app, ctx);
    crate::bounce::show_live_commit(app, ctx);
    check_linked_sessions(app, ctx);
    let Some(mut d) = app.dialogs.open.take() else { return };
    let mut keep = true;
    let mut action: Option<(String, Value)> = None;
    let mut save_first: Option<(String, Value)> = None;
    let confirmed = matches!(d, Dialog::SaveChanges { .. });
    let title = match &d {
        Dialog::NewTracks { .. } => "New Tracks",
        Dialog::Bounce { .. } => "Bounce Mix",
        Dialog::RenameTrack { .. } => "Rename Track",
        Dialog::RenameClip { .. } => "Rename Clip",
        Dialog::RenameMarker { .. } => "Rename Marker",
        Dialog::ClearClips { .. } => "Clear",
        Dialog::PathPrompt { title, .. } | Dialog::Number { title, .. } => title.as_str(),
        Dialog::TempoChange { .. } => "Tempo Change",
        Dialog::Spot(_) => "Spot",
        Dialog::Fades { .. } => "Fades",
        Dialog::StripSilence { .. } => "Strip Silence",
        Dialog::NewFolder { .. } => "Move to New Folder",
        Dialog::ImportSessionData(_) => "Import Session Data",
        Dialog::LinkedChanged { .. } => "Linked Session Changed",
        Dialog::Group { modify: Some(_), .. } => "Modify Group",
        Dialog::Group { .. } => "Create Group",
        Dialog::Conform { .. } => "Conform Song to Tempo",
        Dialog::Session { .. } => "Session Setup",
        Dialog::ScoreSetup { .. } => "Score Setup",
        Dialog::MidiInputDevices { .. } => "MIDI Input Devices",
        Dialog::SaveChanges { then } if then.0 == "session.revert" => "Revert to Saved",
        Dialog::SaveChanges { .. } => "Unsaved Changes",
    }
    .to_string();
    let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
    let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Window::new(egui::RichText::new(&title).font(bold(13.0)))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -80.0))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            match &mut d {
                Dialog::NewTracks { rows } => {
                    ui.set_min_width(900.0);
                    let (mut add, mut remove) = (None, None);
                    let many = rows.len() > 1;
                    for (i, NewTrackRow { count, format, kind, timebase, name }) in rows.iter_mut().enumerate() {
                    ui.push_id(i, |ui| ui.horizontal(|ui| {
                        ui.label("Create");
                        crate::widgets::value_box(ui, |ui| ui.add_sized([46.0, 20.0], egui::DragValue::new(count).range(1..=128)));
                        ui.label("new");
                        egui::ComboBox::from_id_salt("nt_fmt").width(110.0).selected_text(format.as_str()).show_ui(ui, |ui| {
                            for f in openpt_model::ChannelFormat::ALL {
                                ui.selectable_value(format, f.label().to_string(), f.label());
                            }
                        });
                        egui::ComboBox::from_id_salt("nt_kind")
                            .width(130.0)
                            .selected_text(openpt_model::TrackKind::from_id(kind).map_or("Audio Track", |k| k.label()))
                            .show_ui(ui, |ui| {
                                for k in openpt_model::TrackKind::ALL {
                                    if ui.selectable_label(kind == k.id(), k.label()).clicked() {
                                        *kind = k.id().to_string();
                                        *name = k.default_name().to_string();
                                    }
                                }
                            });
                        ui.label("in");
                        egui::ComboBox::from_id_salt("nt_tb")
                            .width(100.0)
                            .selected_text(if timebase == "ticks" { "Ticks" } else { "Samples" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(timebase, "samples".to_string(), "Samples");
                                ui.selectable_value(timebase, "ticks".to_string(), "Ticks");
                            });
                        ui.label("Name:");
                        ui.add(egui::TextEdit::singleline(name).desired_width(220.0));
                        if ui.small_button("+").on_hover_text("Add a row").clicked() {
                            add = Some(i);
                        }
                        if many && ui.small_button("−").on_hover_text("Remove this row").clicked() {
                            remove = Some(i);
                        }
                    }));
                    }
                    if let Some(i) = add {
                        let copy = rows.get(i).cloned().unwrap_or_default();
                        rows.insert(i + 1, copy);
                    }
                    if let Some(i) = remove {
                        rows.remove(i);
                    }
                    if buttons(ui, "Create", enter) {
                        let list: Vec<Value> = rows
                            .iter()
                            .map(|r| json!({"count": r.count, "format": r.format, "kind": r.kind, "timebase": r.timebase, "name": r.name}))
                            .collect();
                        action = Some(("track.new".into(), json!({"rows": list})));
                    }
                }
                Dialog::Bounce { path, format, bit_depth, normalize, dither, realtime } => {
                    ui.horizontal(|ui| {
                        ui.label("File");
                        ui.add(egui::TextEdit::singleline(path).desired_width(420.0));
                        if let Some(pick) = &app.services.pick_save
                            && ui.button("Browse…").clicked()
                            && let Some(p) = pick("file.bounce_mix", path)
                        {
                            *path = p;
                            match std::path::Path::new(path.as_str()).extension().map(|x| x.to_string_lossy().to_lowercase()).as_deref() {
                                Some("wav") => *format = "wav".into(),
                                Some("aif" | "aiff") => *format = "aiff".into(),
                                Some("flac") => *format = "flac".into(),
                                Some("mp3") => *format = "mp3".into(),
                                _ => {}
                            }
                        }
                    });
                    let before = format.clone();
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_label("Format").selected_text(format.to_uppercase()).show_ui(ui, |ui| {
                            for f in ["wav", "aiff", "flac", "mp3"] {
                                ui.selectable_value(format, f.to_string(), f.to_uppercase());
                            }
                        });
                        if format != "mp3" {
                        egui::ComboBox::from_label("Bit Depth").selected_text(format!("{bit_depth}-bit")).show_ui(ui, |ui| {
                            for b in ["16", "24", "32f"] {
                                ui.selectable_value(bit_depth, b.to_string(), format!("{b}-bit"));
                            }
                        });
                        } else {
                            ui.label("320 kb/s");
                        }
                    });
                    if *format != before {
                        let ext = if format == "aiff" { "aif" } else { format.as_str() };
                        *path = std::path::Path::new(path.as_str()).with_extension(ext).to_string_lossy().into_owned();
                    }
                    crate::widgets::checkbox(ui, normalize, "Normalize to -0.1 dBFS");
                    if bit_depth != "32f" && format != "mp3" {
                        crate::widgets::checkbox(ui, dither, "Dither (TPDF)");
                    }
                    crate::widgets::checkbox(ui, realtime, "Real-Time (plays through the audio interface, for outboard gear)");
                    ui.label(
                        egui::RichText::new("Bounces the edit selection, or the whole session when nothing is selected.")
                            .small()
                            .color(Tokens::current().text_dim),
                    );
                    if buttons(ui, "Bounce", enter) {
                        let p = std::path::Path::new(path.as_str()).with_extension(if format == "aiff" { "aif" } else { format.as_str() });
                        let params = json!({"path": p.to_string_lossy(), "format": format, "bit_depth": bit_depth, "normalize": normalize, "dither": *dither && bit_depth != "32f"});
                        if *realtime {
                            crate::bounce::start_realtime(app, &params);
                        } else {
                            crate::bounce::start(app, &params);
                        }
                        keep = false;
                    }
                }
                Dialog::RenameTrack { id, name } => {
                    let mut step = ui.input_mut(|i| {
                        if i.consume_key(egui::Modifiers::COMMAND, egui::Key::ArrowUp) {
                            -1
                        } else if i.consume_key(egui::Modifiers::COMMAND, egui::Key::ArrowDown) {
                            1
                        } else {
                            0
                        }
                    });
                    let field = name_field(ui, name);
                    ui.horizontal(|ui| {
                        if ui.button("◀ Previous").clicked() {
                            step = -1;
                        }
                        if ui.button("Next ▶").clicked() {
                            step = 1;
                        }
                    });
                    if step != 0
                        && let Some((next, next_name)) = neighbour_track(app.engine.session(), *id, step)
                    {
                        let _ = app.run("track.rename", json!({"track": id.0, "name": name}));
                        *id = next;
                        *name = next_name;
                        ui.memory_mut(|m| m.surrender_focus(field));
                    }
                    if buttons(ui, "OK", enter) {
                        action = Some(("track.rename".into(), json!({"track": id.0, "name": name})));
                    }
                }
                Dialog::RenameClip { id, name } => {
                    name_field(ui, name);
                    if buttons(ui, "OK", enter) {
                        action = Some(("clip.rename".into(), json!({"clip": id.0, "name": name})));
                    }
                }
                Dialog::RenameMarker { id, name } => {
                    name_field(ui, name);
                    if buttons(ui, "OK", enter) {
                        action = Some(("markers.edit".into(), json!({"id": id.0, "rename": name})));
                    }
                }
                Dialog::ClearClips { clips, sources, message } => {
                    ui.set_max_width(340.0);
                    ui.label(message.as_str());
                    if buttons(ui, "Remove", enter) {
                        action = Some(("clip.clear".into(), json!({"clips": clips, "sources": sources, "with_clips": true})));
                    }
                }
                Dialog::PathPrompt { cmd, path, key, .. } => {
                    ui.add(egui::TextEdit::singleline(path).desired_width(420.0));
                    if let Some(pick) = &app.services.pick_open
                        && key == "path"
                        && cmd != "session.save_as"
                        && cmd != "session.save_copy"
                        && cmd != "session.save_template"
                        && key != "dir"
                        && ui.button("Browse…").clicked()
                        && let Some(p) = pick(cmd, &[])
                    {
                        *path = p;
                    }
                    if let Some(pick) = &app.services.pick_folder
                        && key == "dir"
                        && ui.button("Browse…").clicked()
                        && let Some(p) = pick(path)
                    {
                        *path = p;
                    }
                    if let Some(pick) = &app.services.pick_save
                        && (cmd == "session.save_as" || cmd == "session.save_copy" || cmd == "session.save_template")
                        && ui.button("Browse…").clicked()
                        && let Some(p) = pick(cmd, path)
                    {
                        *path = p;
                    }
                    let file = openpt_engine::io::session_file(path);
                    let replaces = openpt_engine::io::replaces(&app.engine, cmd, &file);
                    if replaces {
                        ui.label(format!("“{}” already exists. Do you want to replace it?", file.file_name().unwrap_or_default().to_string_lossy()));
                    }
                    if buttons(ui, if replaces { "Replace" } else { "OK" }, enter && !replaces) {
                        let mut v = json!({});
                        v[key.as_str()] = json!(path);
                        if replaces {
                            v["replace"] = json!(true);
                        }
                        action = Some((cmd.clone(), v));
                    }
                }
                Dialog::Number { cmd, key, value, suffix, .. } => {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(value).speed(0.1));
                        ui.label(suffix.as_str());
                    });
                    if buttons(ui, "OK", enter) {
                        let v = if key == "by_seconds" { json!({"by": {"seconds": value}}) } else { json!({key.as_str(): value}) };
                        action = Some((cmd.clone(), v));
                    }
                }
                Dialog::TempoChange { loc, bpm, from_tick, snap_bar } => {
                    egui::Grid::new("tempo_change").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                        ui.label("Location");
                        crate::widgets::value_box(ui, |ui| ui.add_sized([110.0, 20.0], egui::TextEdit::singleline(loc)));
                        ui.end_row();
                        ui.label("BPM");
                        crate::widgets::value_box(ui, |ui| {
                            ui.add_sized([110.0, 20.0], egui::DragValue::new(bpm).speed(0.1).range(5.0..=1000.0).max_decimals(4))
                        });
                        ui.end_row();
                    });
                    ui.checkbox(snap_bar, "Snap to Bar");
                    let tick = parse_bar_beat(app.engine.session(), loc, *snap_bar);
                    if tick.is_none() {
                        ui.colored_label(Color32::from_rgb(230, 120, 100), "Location: bar|beat|tick");
                    }
                    if buttons(ui, "OK", enter)
                        && let Some(tick) = tick
                    {
                        action = Some(("event.tempo".into(), json!({"bpm": *bpm, "tick": tick, "from_tick": *from_tick})));
                    }
                }
                Dialog::Spot(d) => {
                    crate::spot::fields(ui, app.engine.session(), d);
                    if buttons(ui, "OK", enter) {
                        keep = false;
                        action = d.command(app.engine.session());
                    }
                }
                Dialog::Fades { shape, batch: Some(b) } => {
                    let _ = shape;
                    b.ui(ui, &mut app.ui.fade_presets);
                    if buttons(ui, "OK", enter) {
                        let rate = f64::from(app.engine.session().sample_rate.hz());
                        action = Some(("edit.fades_create".into(), b.params(rate)));
                    }
                }
                Dialog::Fades { shape, batch: None } => {
                    for s in openpt_model::FadeShape::ALL {
                        ui.radio_value(shape, s.label().to_lowercase(), s.label());
                    }
                    if buttons(ui, "OK", enter) {
                        action = Some(("edit.fades_create".into(), json!({"shape": shape})));
                    }
                }
                Dialog::StripSilence { threshold, min_ms, pre_ms, post_ms } => {
                    ui.add(egui::Slider::new(threshold, -96.0..=0.0).text("Threshold dB"));
                    ui.add(egui::Slider::new(min_ms, 0.0..=2000.0).text("Min strip duration ms"));
                    ui.add(egui::Slider::new(pre_ms, 0.0..=500.0).text("Clip start pad ms"));
                    ui.add(egui::Slider::new(post_ms, 0.0..=2000.0).text("Clip end pad ms"));
                    if buttons(ui, "Strip", enter) {
                        action = Some((
                            "edit.strip_silence".into(),
                            json!({"threshold_db": threshold, "min_length_ms": min_ms, "pad_before_ms": pre_ms, "pad_after_ms": post_ms}),
                        ));
                    }
                }
                Dialog::ImportSessionData(d) => {
                    if let Some(a) = import_session_data_ui(ui, d, app.engine.session(), enter) {
                        action = Some(a);
                    }
                }
                Dialog::LinkedChanged { path, tracks } => {
                    ui.set_max_width(420.0);
                    let file = std::path::Path::new(path.as_str()).file_name().map_or(path.clone(), |f| f.to_string_lossy().into_owned());
                    ui.label(format!("“{file}” was saved since its tracks were imported ({}). Import its changes?", tracks.join(", ")));
                    ui.label(egui::RichText::new("The tracks of the same name take its new data; what they had stays as alternate playlists.").weak());
                    ui.horizontal(|ui| {
                        if ui.button("Not Now").clicked() {
                            action = Some(("file.update_linked_session".into(), json!({"path": path, "dismiss": true})));
                        }
                        if ui.button("Import Changes").clicked() || enter {
                            action = Some(("file.update_linked_session".into(), json!({"path": path})));
                        }
                    });
                }
                Dialog::NewFolder { name, routing, route, tracks } => {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        let r = ui.text_edit_singleline(name);
                        if !r.has_focus() && ui.memory(|m| m.focused().is_none()) {
                            r.request_focus();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.radio_value(routing, false, "Basic Folder");
                        ui.radio_value(routing, true, "Routing Folder");
                    });
                    ui.add_enabled_ui(*routing, |ui| crate::widgets::checkbox(ui, route, "Route the tracks to the folder"));
                    if buttons(ui, "Create", enter) {
                        action = Some(("track.move_to_new_folder".into(), json!({"tracks": tracks, "name": if name.trim().is_empty() { "Folder" } else { name.trim() }, "routing": routing, "route": route})));
                    }
                }
                Dialog::Group { name, edit, mix, members, modify } => {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(name);
                    });
                    ui.horizontal(|ui| {
                        crate::widgets::checkbox(ui, edit, "Edit group");
                        crate::widgets::checkbox(ui, mix, "Mix group");
                    });
                    ui.label("Tracks");
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        for (_, n, on) in members.iter_mut() {
                            crate::widgets::checkbox(ui, on, n.as_str());
                        }
                    });
                    if buttons(ui, "OK", enter) {
                        let ids: Vec<u64> = members.iter().filter(|m| m.2).map(|m| m.0).collect();
                        action = Some(match modify {
                            Some(g) => ("track.group_modify".into(), json!({"group": g, "name": name, "edit": edit, "mix": mix, "tracks": ids})),
                            None => ("track.group".into(), json!({"name": name, "edit": edit, "mix": mix, "tracks": ids})),
                        });
                    }
                }
                Dialog::Conform { bpm, bar, grid, refs, all_tracks } => {
                    ui.set_min_width(420.0);
                    ui.horizontal(|ui| {
                        ui.label("Tempo");
                        ui.add(egui::TextEdit::singleline(bpm).hint_text("mean of the playing").desired_width(130.0));
                        ui.label("BPM, from bar");
                        crate::widgets::value_box(ui, |ui| ui.add_sized([50.0, 20.0], egui::DragValue::new(bar).range(1..=9999)));
                    });
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_label("Grid").selected_text(grid.as_str()).show_ui(ui, |ui| {
                            for g in ["1/4", "1/8", "1/8t", "1/16", "1/16t", "1/32"] {
                                ui.selectable_value(grid, g.to_string(), g);
                            }
                        });
                    });
                    ui.label("Hits are read on these tracks");
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        for (_, n, on) in refs.iter_mut() {
                            crate::widgets::checkbox(ui, on, n.as_str());
                        }
                    });
                    crate::widgets::checkbox(ui, all_tracks, "Every audio track follows (Elastic Audio is turned on where it is off)");
                    ui.label(
                        egui::RichText::new("The current tempo map places each hit on its beat; the song then plays at the new tempo.")
                            .small()
                            .color(Tokens::current().text_dim),
                    );
                    if buttons(ui, "Conform", enter) {
                        conform(app, bpm, *bar, grid, refs, *all_tracks);
                        keep = false;
                    }
                }
                Dialog::Session { frame_rate, bit_depth } => {
                    egui::ComboBox::from_label("Timecode Rate").selected_text(frame_rate.as_str()).show_ui(ui, |ui| {
                        for r in openpt_time::FrameRate::ALL {
                            ui.selectable_value(frame_rate, r.label().to_string(), r.label());
                        }
                    });
                    egui::ComboBox::from_label("Bit Depth").selected_text(bit_depth.as_str()).show_ui(ui, |ui| {
                        for b in ["16", "24", "32f"] {
                            ui.selectable_value(bit_depth, b.to_string(), b);
                        }
                    });
                    if buttons(ui, "OK", enter) {
                        action = Some(("setup.session".into(), json!({"frame_rate": frame_rate, "bit_depth": bit_depth})));
                    }
                }
                Dialog::ScoreSetup { title, composer, bars_per_system, show_track_names } => {
                    egui::Grid::new("score_setup").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                        ui.label("Title");
                        ui.text_edit_singleline(title);
                        ui.end_row();
                        ui.label("Composer");
                        ui.text_edit_singleline(composer);
                        ui.end_row();
                        ui.label("Bars per system");
                        ui.add(egui::DragValue::new(bars_per_system).range(1..=16));
                        ui.end_row();
                    });
                    crate::widgets::checkbox(ui, show_track_names, "Show track names");
                    if buttons(ui, "OK", enter) {
                        action = Some((
                            "file.score_setup".into(),
                            json!({"title": title, "composer": composer, "bars_per_system": bars_per_system, "show_track_names": show_track_names}),
                        ));
                    }
                }
                Dialog::MidiInputDevices { devices } => {
                    ui.label("MIDI input devices:");
                    if devices.is_empty() {
                        ui.label(egui::RichText::new("No MIDI input devices detected.").color(Tokens::current().text_dim));
                    } else {
                        for (name, enabled) in devices.iter_mut() {
                            crate::widgets::checkbox(ui, enabled, name.as_str());
                        }
                    }
                    if buttons(ui, "OK", enter) {
                        let enable: Vec<String> = devices.iter().filter(|(_, en)| *en).map(|(n, _)| n.clone()).collect();
                        let disable: Vec<String> = devices.iter().filter(|(_, en)| !*en).map(|(n, _)| n.clone()).collect();
                        action = Some(("setup.midi_input_devices".into(), json!({"enable": enable, "disable": disable})));
                    }
                }
                Dialog::SaveChanges { then } => {
                    let name = &app.engine.session().name;
                    let revert = then.0 == "session.revert";
                    if revert || cfg!(target_arch = "wasm32") {
                        let (question, ok) = if revert {
                            (format!("Revert “{name}” to its last saved version?"), "Revert")
                        } else {
                            (format!("Close “{name}” without saving?"), "Don't Save")
                        };
                        ui.label(format!("{question} Your changes will be lost."));
                        if buttons(ui, ok, false) {
                            action = Some(then.clone());
                        }
                    } else {
                        ui.label(format!("Save changes to “{name}” before closing?"));
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("Don't Save").clicked() {
                                action = Some(then.clone());
                            }
                            if buttons(ui, "Save", enter) {
                                save_first = Some(then.clone());
                            }
                        });
                    }
                }
            }
        });
    if esc || ctx.memory(|m| m.data.get_temp::<bool>(egui::Id::new("dlg_cancel")).unwrap_or(false)) {
        keep = false;
        app.dialogs.after_save = None;
        ctx.memory_mut(|m| m.data.remove::<bool>(egui::Id::new("dlg_cancel")));
    }
    if let Some(then) = save_first {
        keep = false;
        if !app.dialogs.open_for_command(&app.engine, "session.save") {
            action = Some(("session.save".into(), json!({})));
        }
        app.dialogs.after_save = Some(then);
    }
    if let Some((cmd, params)) = &action
        && !confirmed
        && ask_to_save(app, cmd, params)
    {
        keep = false;
        action = None;
    }
    let saving = action.as_ref().is_some_and(|(cmd, _)| cmd.starts_with("session.save"));
    if let Some((cmd, params)) = action {
        keep = false;
        if cmd == "file.import_session_data"
            && params.get("tracks").is_none()
            && let Some(path) = params["path"].as_str()
        {
            if let Err(e) = app.dialogs.open_import_session_data(&mut app.engine, path) {
                app.ui.status = e;
            }
            return;
        }
        if cmd == "file.import_audio"
            && let Some(path) = params["path"].as_str()
        {
            let at = app.engine.session().edit.selection.start;
            crate::import_jobs::start(app, vec![path.to_string()], at);
            return;
        }
        match app.run(&cmd, params) {
            Ok(v) if cmd == "file.export_tracks_audio" => {
                app.ui.status = format!("Exported {} track(s) as audio files", v["files"].as_array().map_or(0, Vec::len));
            }
            Ok(v) if cmd == "file.export_clips" => {
                app.ui.status = format!("Exported {} clip(s) to {}", v["written"], v["dir"].as_str().unwrap_or_default());
            }
            Ok(_) => {}
            Err(e) => app.ui.status = e,
        }
        if cmd == "setup.midi_input_devices" {
            app.sync_midi_devices();
        }
    }
    if saving
        && let Some((then, params)) = app.dialogs.after_save.take()
        && !app.engine.is_dirty()
        && let Err(e) = app.run(&then, params)
    {
        app.ui.status = e;
    }
    if keep {
        app.dialogs.open = Some(d);
    }
}

fn check_linked_sessions(app: &mut SoundApp, ctx: &egui::Context) {
    if app.engine.session().linked_sessions.is_empty() {
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_secs(5));
    let id = egui::Id::new("linked_sessions_checked");
    let now = ctx.input(|i| i.time);
    if app.dialogs.open.is_some() || ctx.data(|d| d.get_temp::<f64>(id)).is_some_and(|t| now - t < 5.0) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(id, now));
    let Ok(v) = app.engine.execute("file.linked_sessions", &json!({})) else { return };
    if let Some(c) = v["changed"].as_array().and_then(|a| a.first()) {
        let tracks = c["tracks"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
        app.dialogs.open = Some(Dialog::LinkedChanged { path: c["path"].as_str().unwrap_or_default().to_string(), tracks });
    }
}

#[derive(Debug, Clone)]
pub struct ImportRow {
    pub name: String,
    pub kind: String,
    pub folder: Option<String>,
    pub clips: u64,
    pub on: bool,
    pub dest: String,
}

#[derive(Debug, Clone)]
pub struct ImportSessionData {
    pub path: String,
    pub name: String,
    pub rows: Vec<ImportRow>,
    pub data: [bool; openpt_engine::cmd::TRACK_DATA.len()],
    pub markers: bool,
    pub tempo: bool,
    pub link: bool,
    pub has_markers: bool,
    pub has_tempo: bool,
}

const TRACK_DATA_LABELS: [&str; openpt_engine::cmd::TRACK_DATA.len()] =
    ["Clips and media", "Alternate playlists", "Plug-ins", "Sends", "I/O", "Volume", "Pan", "Mute / Solo", "Automation", "Comments"];

fn import_session_data_ui(ui: &mut egui::Ui, d: &mut ImportSessionData, s: &openpt_model::Session, enter: bool) -> Option<(String, Value)> {
    ui.set_min_width(640.0);
    ui.label(egui::RichText::new(format!("Source: {}", if d.name.is_empty() { &d.path } else { &d.name })).strong());
    let here: Vec<String> = s.tracks.iter().filter(|t| !matches!(t.kind, openpt_model::TrackKind::Master | openpt_model::TrackKind::Video)).map(|t| t.name.clone()).collect();
    ui.horizontal(|ui| {
        if ui.button("All").clicked() {
            d.rows.iter_mut().for_each(|r| r.on = true);
        }
        if ui.button("None").clicked() {
            d.rows.iter_mut().for_each(|r| r.on = false);
        }
        if ui.button("Match Tracks").on_hover_text("Each track onto the track of the same name here, when there is one").clicked() {
            for r in &mut d.rows {
                r.dest = if here.contains(&r.name) { r.name.clone() } else { String::new() };
            }
        }
        if ui.button("All New").clicked() {
            d.rows.iter_mut().for_each(|r| r.dest.clear());
        }
    });
    egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
        egui::Grid::new("isd_tracks").num_columns(3).striped(true).spacing(vec2(12.0, 4.0)).show(ui, |ui| {
            ui.label(egui::RichText::new("Source track").strong());
            ui.label(egui::RichText::new("Clips").strong());
            ui.label(egui::RichText::new("Destination").strong());
            ui.end_row();
            for (i, r) in d.rows.iter_mut().enumerate() {
                let label = match &r.folder {
                    Some(f) => format!("{f} › {}", r.name),
                    None => r.name.clone(),
                };
                crate::widgets::checkbox(ui, &mut r.on, label);
                ui.label(if r.kind == "folder" { "folder".to_string() } else { r.clips.to_string() });
                ui.add_enabled_ui(r.on, |ui| {
                    egui::ComboBox::from_id_salt(("isd_dest", i)).width(200.0).selected_text(if r.dest.is_empty() { "New Track" } else { r.dest.as_str() }).show_ui(
                        ui,
                        |ui| {
                            ui.selectable_value(&mut r.dest, String::new(), "New Track");
                            for n in &here {
                                ui.selectable_value(&mut r.dest, n.clone(), n.as_str());
                            }
                        },
                    );
                });
                ui.end_row();
            }
        });
    });
    ui.separator();
    ui.label(egui::RichText::new("Track data to import").strong());
    egui::Grid::new("isd_data").num_columns(4).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
        for (k, (on, label)) in d.data.iter_mut().zip(TRACK_DATA_LABELS).enumerate() {
            crate::widgets::checkbox(ui, on, label);
            if k % 4 == 3 {
                ui.end_row();
            }
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.add_enabled_ui(d.has_markers, |ui| crate::widgets::checkbox(ui, &mut d.markers, "Markers / Memory Locations"));
        ui.add_enabled_ui(d.has_tempo, |ui| crate::widgets::checkbox(ui, &mut d.tempo, "Tempo / Meter map"));
    });
    crate::widgets::checkbox(ui, &mut d.link, "Link to this session: when it is saved again, offer to import its changes");
    let picked: Vec<&ImportRow> = d.rows.iter().filter(|r| r.on).collect();
    let ok = !picked.is_empty() || d.markers || d.tempo;
    let mut go = false;
    ui.add_enabled_ui(ok, |ui| go = buttons(ui, "Import", enter && ok));
    if !go || !ok {
        return None;
    }
    let tracks: Vec<&str> = picked.iter().map(|r| r.name.as_str()).collect();
    let dest: serde_json::Map<String, Value> = picked.iter().filter(|r| !r.dest.is_empty()).map(|r| (r.name.clone(), json!(r.dest))).collect();
    let data: serde_json::Map<String, Value> = openpt_engine::cmd::TRACK_DATA.iter().zip(d.data).map(|(k, on)| ((*k).to_string(), json!(on))).collect();
    Some((
        "file.import_session_data".into(),
        json!({"path": d.path, "tracks": tracks, "dest": dest, "data": data, "markers": d.markers, "tempo": d.tempo, "link": d.link}),
    ))
}

fn buttons(ui: &mut egui::Ui, ok: &str, enter: bool) -> bool {
    let mut pressed = enter;
    ui.separator();
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width().max(260.0), 24.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.add(egui::Button::new(egui::RichText::new(ok).strong()).fill(Tokens::current().accent).min_size(egui::vec2(88.0, 22.0))).clicked() {
            pressed = true;
        }
        if ui.add(egui::Button::new("Cancel").min_size(egui::vec2(88.0, 22.0))).clicked() {
            ui.ctx().memory_mut(|m| m.data.insert_temp(egui::Id::new("dlg_cancel"), true));
        }
    });
    pressed
}

fn name_field(ui: &mut egui::Ui, name: &mut String) -> egui::Id {
    let mut out = egui::TextEdit::singleline(name).show(ui);
    if !out.response.has_focus() {
        out.response.request_focus();
        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(name.chars().count()));
        out.state.cursor.set_char_range(Some(all));
        out.state.store(ui.ctx(), out.response.id);
    }
    out.response.id
}

fn neighbour_track(s: &openpt_model::Session, id: TrackId, step: i32) -> Option<(TrackId, String)> {
    let visible: Vec<&openpt_model::Track> = s.tracks.iter().filter(|t| !t.hidden).collect();
    let i = visible.iter().position(|t| t.id == id)?;
    let j = i.checked_add_signed(step as isize)?;
    visible.get(j).map(|t| (t.id, t.name.clone()))
}

pub(crate) fn parse_bar_beat(s: &openpt_model::Session, text: &str, bar: bool) -> Option<i64> {
    let mut n = text.split(|c: char| c == '|' || c == '.' || c.is_whitespace()).filter(|x| !x.is_empty()).map(str::parse::<i64>);
    let b = n.next()?.ok()?;
    let beat = n.next().transpose().ok()?.unwrap_or(1);
    let tick = n.next().transpose().ok()?.unwrap_or(0);
    if b < 1 || beat < 1 || tick < 0 {
        return None;
    }
    let bb = if bar { openpt_time::BarBeat { bar: b, beat: 1, tick: 0 } } else { openpt_time::BarBeat { bar: b, beat, tick } };
    Some(s.tempo.tick_at_bar_beat(bb))
}

pub(crate) fn bar_beat_text(s: &openpt_model::Session, tick: i64) -> String {
    let bb = s.tempo.bar_beat_at_tick(tick);
    format!("{}|{}|{:03}", bb.bar, bb.beat, bb.tick)
}

fn audiosuite_window(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(process) = app.ui.audiosuite.clone() else { return };
    let mut open = true;
    let title = openpt_dsp::plugin_info(&process).map_or_else(|| process.replace('_', " "), |p| p.name.to_string());
    egui::Window::new(format!("AudioSuite · {title}")).open(&mut open).default_width(360.0).show(ctx, |ui| {
        let key = egui::Id::new(("as_params", process.clone()));
        let mut params: std::collections::BTreeMap<String, f32> = ui.ctx().memory(|m| m.data.get_temp(key)).unwrap_or_default();
        if let Some(info) = openpt_dsp::plugin_info(&process).filter(|i| crate::open_ui::handles(i.id)) {
            let mut edits: Vec<(String, f32)> = Vec::new();
            crate::open_ui::panel(ui, info, &params, None, egui::Id::new(("as-open", info.id)), &mut |id, v| edits.push((id.to_string(), v)));
            params.extend(edits);
        } else if let Some(info) = openpt_dsp::plugin_info(&process) {
            for p in info.params {
                let v = params.entry(p.id.to_string()).or_insert(p.default);
                ui.add(egui::Slider::new(v, p.min..=p.max).text(p.name));
            }
        } else {
            match process.as_str() {
                "normalize" => {
                    let v = params.entry("target_db".into()).or_insert(-0.1);
                    ui.add(egui::Slider::new(v, -24.0..=0.0).text("Peak dB"));
                }
                "time_stretch" => {
                    let v = params.entry("ratio".into()).or_insert(1.0);
                    ui.add(egui::Slider::new(v, 0.25..=4.0).text("Length ratio"));
                }
                "pitch_shift" => {
                    let v = params.entry("semitones".into()).or_insert(0.0);
                    ui.add(egui::Slider::new(v, -24.0..=24.0).text("Semitones"));
                }
                "varispeed" => {
                    let v = params.entry("speed".into()).or_insert(1.0);
                    ui.add(egui::Slider::new(v, 0.25..=4.0).text("Speed"));
                }
                _ => {
                    ui.label("No parameters.");
                }
            }
        }
        ui.ctx().memory_mut(|m| m.data.insert_temp(key, params.clone()));
        let hint = if openpt_dsp::is_generator(&process) {
            "Fills the selection on the selected tracks with a new clip."
        } else {
            "Processes the selected clips and replaces them with rendered audio."
        };
        ui.label(egui::RichText::new(hint).small());
        if ui.button(egui::RichText::new("Render").strong()).clicked() {
            let p: serde_json::Map<String, Value> = params.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
            let _ = app.run("audiosuite.process", json!({"process": process, "params": p}));
        }
    });
    if !open {
        app.ui.audiosuite = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_commands_get_path_prompts_with_sensible_names() {
        let e = openpt_engine::demo::demo_engine();
        let mut d = Dialogs::default();
        for (id, ext) in [("file.export_sibelius", ".musicxml"), ("file.print_score", ".svg"), ("file.export_clip_groups", ".scgrp")] {
            assert!(takes_path(id), "{id}");
            assert!(d.open_for_command(&e, id), "{id}");
            match d.open.take() {
                Some(Dialog::PathPrompt { cmd, path, key, .. }) => {
                    assert_eq!(cmd, id);
                    assert_eq!(key, "path");
                    assert!(path.ends_with(ext), "{path}");
                }
                other => panic!("{id}: {other:?}"),
            }
        }
        assert!(!takes_path("file.send_to_sibelius"));
        assert!(!takes_path("edit.copy"));
        assert!(d.open_for_command(&e, "file.score_setup"));
        assert!(matches!(d.open, Some(Dialog::ScoreSetup { bars_per_system: 4, .. })));
    }

    #[test]
    fn clip_rename_opens_the_rename_dialog_for_the_selected_clip() {
        let mut e = openpt_engine::demo::demo_engine();
        let mut d = Dialogs::default();
        e.execute("edit.select_none", &json!({})).unwrap();
        assert!(!d.open_for_command(&e, "clip.rename"));
        let clip = e.session().track_by_name("Kick").unwrap().clips()[0].clone();
        e.execute("edit.select", &json!({"clips": [clip.id.0]})).unwrap();
        assert!(d.open_for_command(&e, "clip.rename"));
        assert!(matches!(d.open, Some(Dialog::RenameClip { id, ref name }) if id == clip.id && *name == clip.name));
    }

    #[test]
    fn tempo_change_dialog_places_and_moves_a_change_at_a_bar_beat_location() {
        let mut e = openpt_engine::demo::demo_engine();
        let s = e.session().clone();
        let t5 = parse_bar_beat(&s, "5|1|000", false).unwrap();
        assert_eq!(bar_beat_text(&s, t5), "5|1|000");
        assert_eq!(parse_bar_beat(&s, "5 3", true), Some(t5));
        assert!(parse_bar_beat(&s, "x", false).is_none());
        e.execute("event.tempo", &json!({"bpm": 133.5, "tick": t5})).unwrap();
        let t9 = parse_bar_beat(e.session(), "9", false).unwrap();
        e.execute("event.tempo", &json!({"bpm": 90.0, "tick": t9, "from_tick": t5})).unwrap();
        let ticks: Vec<(i64, f64)> = e.session().tempo.tempos().iter().map(|x| (x.tick, x.bpm)).collect();
        assert!(ticks.contains(&(t9, 90.0)) && !ticks.iter().any(|x| x.0 == t5), "{ticks:?}");
    }

    #[test]
    fn clear_always_asks_and_names_the_timeline_clips_that_go_too() {
        let mut e = openpt_engine::demo::demo_engine();
        let file = |e: &Engine, name: &str| e.session().sources.iter().find(|f| f.name == name).map(|f| f.id).unwrap();
        let (kick, hats) = (file(&e, "Kick"), file(&e, "Hats"));
        let kick_clips: Vec<ClipId> = e.session().clips_using(kick).map(|c| c.id).collect();
        let snare = e.session().track_by_name("Snare").unwrap().clips()[0].id;
        let message = |d: &Dialogs| match &d.open {
            Some(Dialog::ClearClips { message, .. }) => message.clone(),
            other => panic!("{other:?}"),
        };
        let mut d = Dialogs::default();
        assert!(!d.open_for_command(&e, "clip.clear"));
        e.execute("edit.select", &json!({"clips": [snare]})).unwrap();
        assert!(d.open_for_command(&e, "clip.clear"));
        assert_eq!(message(&d), "Remove the clip \"Snare\" from the session?");
        e.execute("edit.select", &json!({"sources": [kick]})).unwrap();
        assert!(d.open_for_command(&e, "clip.clear"));
        assert!(matches!(&d.open, Some(Dialog::ClearClips { clips, sources, .. }) if clips.is_empty() && *sources == vec![kick]));
        assert_eq!(message(&d), "Remove the file \"Kick\" from the session? It is used by 3 clips on the timeline, which are removed too.");
        assert!(d.open_clear(&e, kick_clips.clone(), vec![kick]));
        assert_eq!(message(&d), "Remove 3 clips and 1 file from the session?");
        assert!(d.open_clear(&e, kick_clips, vec![kick, hats]));
        assert_eq!(message(&d), "Remove 3 clips and 2 files from the session? \"Hats\" is used by a clip on the timeline, which is removed too.");
        e.session_mut().sources.iter_mut().for_each(|f| f.unsaved = false);
        assert!(d.open_clear(&e, vec![], vec![kick, hats]));
        assert_eq!(
            message(&d),
            "Remove 2 files from the session? They are used by 4 clips on the timeline, which are removed too. The files stay on disk."
        );
    }

    #[test]
    fn midi_input_devices_dialog_opens_and_reports_name() {
        let e = openpt_engine::demo::demo_engine();
        let mut d = Dialogs::default();
        assert!(d.open_for_command(&e, "setup.midi_input_devices"));
        assert_eq!(d.open_name(), Some("midi_input_devices"));
        assert!(matches!(d.open, Some(Dialog::MidiInputDevices { .. })));
    }
}

fn conform(app: &mut SoundApp, bpm: &str, bar: i64, grid: &str, refs: &[(u64, String, bool)], all_tracks: bool) {
    let s = app.engine.session_arc();
    let ref_tracks: Vec<TrackId> = refs.iter().filter(|r| r.2).map(|r| TrackId(r.0)).collect();
    if ref_tracks.is_empty() {
        app.ui.status = "Conform Song to Tempo: tick at least one track to read the hits from".into();
        return;
    }
    let drums: Vec<TrackId> = s
        .groups
        .iter()
        .filter(|g| g.edit && g.members.iter().any(|m| ref_tracks.contains(m)))
        .flat_map(|g| g.members.clone())
        .chain(ref_tracks.iter().copied())
        .collect();
    let audio: Vec<&openpt_model::Track> = s.tracks.iter().filter(|t| t.kind == openpt_model::TrackKind::Audio).collect();
    let elastic = |t: &openpt_model::Track| t.elastic.as_deref().is_some_and(|a| a != "none");
    if all_tracks {
        let (mut rhythmic, mut poly) = (Vec::new(), Vec::new());
        for t in audio.iter().filter(|t| !elastic(t)) {
            if drums.contains(&t.id) { rhythmic.push(t.id.0) } else { poly.push(t.id.0) }
        }
        for (ids, algo) in [(rhythmic, "rhythmic"), (poly, "polyphonic")] {
            if !ids.is_empty() {
                let _ = app.run("track.elastic", json!({"tracks": ids, "algorithm": algo}));
            }
        }
    }
    let s = app.engine.session_arc();
    let clips_of = |pred: &dyn Fn(&openpt_model::Track) -> bool| -> Vec<u64> {
        s.tracks.iter().filter(|t| t.kind == openpt_model::TrackKind::Audio && pred(t)).flat_map(|t| t.clips().iter().map(|c| c.id.0)).collect()
    };
    let reference = clips_of(&|t| ref_tracks.contains(&t.id));
    let clips = clips_of(&|t| elastic(t));
    let grid_ticks = match grid {
        "1/4" => 960,
        "1/8" => 480,
        "1/8t" => 320,
        "1/16t" => 160,
        "1/32" => 120,
        _ => 240,
    };
    let from_tick = s.tempo.bar_start_tick(bar.max(1));
    let mut p = json!({"reference": reference, "clips": clips, "from_tick": from_tick, "grid": grid_ticks});
    if let Ok(b) = bpm.trim().replace(',', ".").parse::<f64>() {
        p["bpm"] = json!(b);
    }
    match app.run("clip.warp_conform", p) {
        Ok(v) => app.ui.status = format!("Conformed {} clips to {:.2} BPM ({} points)", v["clips"], v["bpm"].as_f64().unwrap_or(0.0), v["points"]),
        Err(e) => app.ui.status = e,
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BatchFades {
    pub in_shape: String,
    pub out_shape: String,
    pub xfade_shape: String,
    pub fade_in: bool,
    pub fade_out: bool,
    pub crossfade: bool,
    pub length_ms: f32,
    pub placement: String,
    pub create: bool,
    pub adjust: bool,
}

impl Default for BatchFades {
    fn default() -> Self {
        BatchFades {
            in_shape: "equal power".into(),
            out_shape: "equal power".into(),
            xfade_shape: "equal power".into(),
            fade_in: true,
            fade_out: true,
            crossfade: true,
            length_ms: 10.0,
            placement: "centered".into(),
            create: true,
            adjust: true,
        }
    }
}

impl BatchFades {
    fn wanted(s: &openpt_model::Session) -> Option<BatchFades> {
        let r = s.edit.selection;
        let edges: usize = s
            .tracks
            .iter()
            .filter(|t| s.edit.selected_tracks.contains(&t.id))
            .flat_map(|t| t.clips().iter())
            .map(|c| usize::from(r.contains(c.start)) + usize::from(r.start < c.end() && r.end >= c.end()))
            .sum();
        (edges > 1).then(BatchFades::default)
    }

    fn params(&self, rate: f64) -> Value {
        json!({
            "batch": true,
            "in_shape": self.in_shape,
            "out_shape": self.out_shape,
            "xfade_shape": self.xfade_shape,
            "fade_in": self.fade_in,
            "fade_out": self.fade_out,
            "crossfade": self.crossfade,
            "length": ((f64::from(self.length_ms) / 1000.0 * rate).round() as i64).max(1),
            "placement": self.placement,
            "create": self.create,
            "adjust": self.adjust,
        })
    }

    fn ui(&mut self, ui: &mut egui::Ui, presets: &mut [Option<BatchFades>; 5]) {
        let shapes = |ui: &mut egui::Ui, id: &str, on: &mut bool, label: &str, shape: &mut String| {
            ui.horizontal(|ui| {
                ui.checkbox(on, label);
                egui::ComboBox::from_id_salt(id).selected_text(shape.clone()).show_ui(ui, |ui| {
                    for s in openpt_model::FadeShape::ALL {
                        ui.selectable_value(shape, s.label().to_lowercase(), s.label());
                    }
                });
            });
        };
        shapes(ui, "bf_in", &mut self.fade_in, "Fade In", &mut self.in_shape);
        shapes(ui, "bf_x", &mut self.crossfade, "Crossfade", &mut self.xfade_shape);
        shapes(ui, "bf_out", &mut self.fade_out, "Fade Out", &mut self.out_shape);
        ui.horizontal(|ui| {
            ui.label("Placement:");
            for (v, l) in [("pre", "Pre-Splice"), ("centered", "Centered"), ("post", "Post-Splice")] {
                ui.radio_value(&mut self.placement, v.to_string(), l);
            }
        });
        ui.add(egui::Slider::new(&mut self.length_ms, 0.1..=10_000.0).logarithmic(true).text("Length (ms)"));
        ui.checkbox(&mut self.create, "Create new fades");
        ui.checkbox(&mut self.adjust, "Adjust existing fades");
        ui.horizontal(|ui| {
            ui.label("Presets:");
            for (i, p) in presets.iter_mut().enumerate() {
                let b = ui.add(egui::Button::new((i + 1).to_string()).selected(p.as_ref() == Some(self)));
                if b.clicked() {
                    match (ui.input(|i| i.modifiers.command), p.as_ref()) {
                        (false, Some(saved)) => *self = saved.clone(),
                        _ => *p = Some(self.clone()),
                    }
                }
                b.on_hover_text(if p.is_some() { "Click to recall, Ctrl-click to store" } else { "Click to store" });
            }
        });
    }
}
