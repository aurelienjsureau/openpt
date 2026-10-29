use crate::SoundApp;
use openpt_model::{LearnSource, LearnTarget, TrackId};
use serde_json::json;

pub const KIND: &str = "MIDI Learn";

#[derive(Default)]
pub struct Learn {
    #[cfg(not(target_arch = "wasm32"))]
    port: Option<(String, openpt_playback::surface::LearnPort)>,
    failed: Option<(String, f64, String)>,
    pub armed: Option<LearnTarget>,
    held: std::collections::HashMap<LearnSource, bool>,
    merging: bool,
    menu: Option<(LearnTarget, egui::Pos2)>,
    pub last_param: Option<(TrackId, usize, String)>,
}

impl Learn {
    pub fn error(&self) -> Option<&str> {
        self.failed.as_ref().map(|f| f.2.as_str())
    }
}

fn input(app: &SoundApp) -> Option<String> {
    app.ui.peripherals.iter().find(|c| c.kind == KIND && !c.input.is_empty()).map(|c| c.input.clone())
}

pub fn label(app: &SoundApp, t: &LearnTarget) -> String {
    let s = app.engine.session();
    let name = |id: &TrackId| s.track(*id).map_or_else(|| "?".to_string(), |t| t.name.clone());
    match t {
        LearnTarget::Volume { track } => format!("{} · Volume", name(track)),
        LearnTarget::Pan { track, index } => format!("{} · Pan{}", name(track), if *index > 0 { " R" } else { "" }),
        LearnTarget::Mute { track } => format!("{} · Mute", name(track)),
        LearnTarget::Solo { track } => format!("{} · Solo", name(track)),
        LearnTarget::RecordArm { track } => format!("{} · Record", name(track)),
        LearnTarget::SendLevel { track, slot } => format!("{} · Send {}", name(track), char::from(b'A' + *slot as u8)),
        LearnTarget::PluginParam { track, slot, param } => {
            let plugin = s.track(*track).and_then(|t| t.mixer.inserts.get(*slot).cloned().flatten());
            let pname = plugin
                .as_ref()
                .and_then(|i| crate::mix_window::plugin_info(&i.plugin))
                .and_then(|info| info.params.iter().find(|p| p.id == param).map(|p| format!("{} {}", info.name, p.name)))
                .unwrap_or_else(|| param.clone());
            format!("{} · {pname}", name(track))
        }
        LearnTarget::Transport { action } => format!("Transport · {action}"),
    }
}

fn apply(app: &mut SoundApp, target: &LearnTarget, v: f32, rising: bool) {
    let merge = |t: &TrackId, what: &str| format!("learn:{what}:{}", t.0);
    let r = match target {
        LearnTarget::Volume { track } => {
            let db = openpt_model::fader_pos_to_db(v).clamp(-144.0, 12.0);
            app.engine.execute_merged("mix.volume", &json!({"track": track.0, "db": db}), &merge(track, "vol")).map(|_| ()).map_err(|e| e.to_string())
        }
        LearnTarget::Pan { track, index } => app
            .engine
            .execute_merged("mix.pan", &json!({"tracks": [track.0], "pan": v * 2.0 - 1.0, "index": index}), &merge(track, "pan"))
            .map(|_| ())
            .map_err(|e| e.to_string()),
        LearnTarget::SendLevel { track, slot } => {
            let db = openpt_model::fader_pos_to_db(v).clamp(-144.0, 12.0);
            app.engine
                .execute_merged("mix.send_level", &json!({"track": track.0, "slot": slot, "db": db}), &merge(track, &format!("send{slot}")))
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        LearnTarget::PluginParam { track, slot, param } => {
            let range = app
                .engine
                .session()
                .track(*track)
                .and_then(|t| t.mixer.inserts.get(*slot).cloned().flatten())
                .and_then(|i| crate::mix_window::plugin_info(&i.plugin))
                .and_then(|info| info.params.iter().find(|p| p.id == param).map(|p| (p.min, p.max)));
            match range {
                Some((lo, hi)) => {
                    let x = lo + (hi - lo) * v;
                    if let Some(pl) = &app.player {
                        pl.editor_set_param(*track, *slot, param, x);
                    }
                    app.engine
                        .execute_merged("mix.insert_param", &json!({"track": track.0, "slot": slot, "param": param, "value": x}), &merge(track, param))
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                }
                None => Ok(()),
            }
        }
        LearnTarget::Mute { track } if rising => app.run("mix.mute", json!({"track": track.0})).map(|_| ()),
        LearnTarget::Solo { track } if rising => app.run("mix.solo", json!({"track": track.0})).map(|_| ()),
        LearnTarget::RecordArm { track } if rising => app.run("mix.record_arm", json!({"track": track.0})).map(|_| ()),
        LearnTarget::Transport { action } if rising => app.run(action, json!({})).map(|_| ()),
        _ => Ok(()),
    };
    if let Err(e) = r {
        app.ui.status = e;
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run(app: &mut SoundApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    let want = input(app);
    if app.learn.port.as_ref().map(|p| &p.0) != want.as_ref() {
        app.learn.port = None;
    }
    if let Some(name) = want.as_ref()
        && app.learn.port.is_none()
        && app.learn.failed.as_ref().is_none_or(|(n, at, _)| n != name || now - at > 5.0)
    {
        match openpt_playback::surface::LearnPort::open(name) {
            Ok(p) => {
                app.learn.port = Some((name.clone(), p));
                app.learn.failed = None;
            }
            Err(e) => app.learn.failed = Some((name.clone(), now, e)),
        }
    }
    let Some((_, port)) = app.learn.port.as_mut() else { return };
    let msgs = port.poll();
    ctx.request_repaint_after(std::time::Duration::from_millis(30));
    if msgs.is_empty() {
        if app.learn.merging {
            app.engine.end_merge();
            app.learn.merging = false;
        }
        return;
    }
    for m in msgs {
        let Some((source, v, on)) = LearnSource::parse(&m) else { continue };
        let was = app.learn.held.insert(source, on).unwrap_or(false);
        if let Some(target) = app.learn.armed.take() {
            let r = app.run("setup.midi_learn", json!({"action": "add", "source": source, "target": target}));
            app.ui.status = match r {
                Ok(_) => format!("MIDI Learn: {} → {}", source.label(), label(app, &target)),
                Err(e) => e,
            };
            continue;
        }
        let targets: Vec<LearnTarget> = app.engine.session().midi_learn.iter().filter(|x| x.source == source).map(|x| x.target.clone()).collect();
        for t in targets {
            if !t.is_toggle() {
                app.learn.merging = true;
            }
            apply(app, &t, v, on && !was);
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn run(_: &mut SoundApp, _: &egui::Context) {}

pub fn menu_items(app: &mut SoundApp, ui: &mut egui::Ui, target: &LearnTarget) {
    let bound: Vec<String> = app.engine.session().midi_learn.iter().filter(|m| m.target == *target).map(|m| m.source.label()).collect();
    let armed = app.learn.armed.as_ref() == Some(target);
    let text = if armed { "MIDI Learn (waiting…)" } else { "MIDI Learn" };
    if ui.button(text).on_hover_text("Move a knob, fader or key on the controller set up in Setup > Peripherals").clicked() {
        app.learn.armed = if armed { None } else { Some(target.clone()) };
        if input(app).is_none() {
            app.ui.status = "MIDI Learn: choose an input in Setup > Peripherals > MIDI Controllers (type “MIDI Learn”)".into();
        }
        ui.close();
    }
    if !bound.is_empty() && ui.button(format!("Forget MIDI ({})", bound.join(", "))).clicked() {
        let _ = app.run("setup.midi_learn", json!({"action": "remove", "target": target}));
        ui.close();
    }
}

pub fn on_right_click(app: &mut SoundApp, ui: &egui::Ui, rect: egui::Rect, target: LearnTarget) {
    if ui.rect_contains_pointer(rect)
        && let Some(pos) = ui.input(|i| i.pointer.secondary_clicked().then(|| i.pointer.interact_pos()).flatten())
    {
        app.learn.menu = Some((target, pos));
    }
}

pub fn popup(app: &mut SoundApp, ctx: &egui::Context) {
    let Some((target, pos)) = app.learn.menu.clone() else { return };
    let mut close = false;
    let area = egui::Area::new(egui::Id::new("midi_learn_menu")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_min_width(180.0);
            ui.weak(label(app, &target));
            ui.separator();
            let before = (app.learn.armed.clone(), app.engine.session().midi_learn.len());
            menu_items(app, ui, &target);
            close = before != (app.learn.armed.clone(), app.engine.session().midi_learn.len());
        });
    });
    let clicked_away = ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() && !ctx.input(|i| i.pointer.secondary_clicked());
    if close || clicked_away || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.learn.menu = None;
    }
}

pub fn window_rows(app: &mut SoundApp, ui: &mut egui::Ui) {
    let maps = app.engine.session().midi_learn.clone();
    if let Some(e) = app.learn.error() {
        ui.colored_label(egui::Color32::from_rgb(230, 120, 100), format!("MIDI Learn: {e}"));
    }
    if maps.is_empty() {
        ui.weak("MIDI Learn: right-click a fader, pan, mute, solo, record, send level or plug-in parameter, choose MIDI Learn and move a control.");
        return;
    }
    ui.strong("MIDI Learn mappings (this session)");
    let mut remove = None;
    egui::Grid::new("learn_maps").num_columns(3).spacing(egui::vec2(12.0, 4.0)).show(ui, |ui| {
        for (i, m) in maps.iter().enumerate() {
            ui.label(m.source.label());
            ui.label(label(app, &m.target));
            if ui.small_button("Remove").clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        let _ = app.run("setup.midi_learn", json!({"action": "remove", "index": i}));
    }
}

pub fn insert_menu_items(app: &mut SoundApp, ui: &mut egui::Ui, track: TrackId, slot: usize) {
    let last = app.learn.last_param.clone().filter(|(t, s, _)| *t == track && *s == slot);
    let learned: Vec<LearnTarget> = app
        .engine
        .session()
        .midi_learn
        .iter()
        .filter(|m| matches!(&m.target, LearnTarget::PluginParam { track: t, slot: s, .. } if *t == track && *s == slot))
        .map(|m| m.target.clone())
        .collect();
    ui.menu_button("MIDI Learn", |ui| {
        match last {
            Some((_, _, param)) => menu_items(app, ui, &LearnTarget::PluginParam { track, slot, param }),
            None => {
                ui.weak("Move a parameter in the plug-in first:\nthe last one touched gets learned.");
            }
        }
        if !learned.is_empty() {
            ui.separator();
            for t in learned {
                ui.horizontal(|ui| {
                    ui.label(label(app, &t));
                    if ui.small_button("Forget").clicked() {
                        let _ = app.run("setup.midi_learn", json!({"action": "remove", "target": t}));
                    }
                });
            }
        }
    });
}
