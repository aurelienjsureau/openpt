use crate::widgets::FitScreen;
use crate::SoundApp;
use crate::theme::{Tokens, bold, regular};
use egui::{Align2, Color32, Sense, Stroke, pos2, vec2};
use openpt_model::{Speaker, TrackKind};
use serde_json::{Value, json};

const AUTO_UPDATE_SECS: f64 = 2.0;

#[derive(Debug, Clone)]
pub struct ExtraState {
    pub show_renderer: bool,
    pub show_trim_downmix: bool,
    pub show_ui_customization: bool,
    pub auto_update_config: bool,
    pub ui_scale: f32,
    pub reduce_motion: bool,
    pub tooltips: bool,
    last_auto_update: f64,
    applied: Option<(f32, bool)>,
}

impl Default for ExtraState {
    fn default() -> Self {
        Self {
            show_renderer: false,
            show_trim_downmix: false,
            show_ui_customization: false,
            auto_update_config: false,
            ui_scale: 1.0,
            reduce_motion: false,
            tooltips: true,
            last_auto_update: 0.0,
            applied: None,
        }
    }
}

pub fn run(app: &mut SoundApp, id: &str, p: &Value) -> Option<Value> {
    let flag = |v: &mut bool| {
        *v = p.get("value").and_then(Value::as_bool).unwrap_or(!*v);
        json!({"value": *v})
    };
    Some(match id {
        "window.renderer" => flag(&mut app.extra.show_renderer),
        "window.trim_downmix" => flag(&mut app.extra.show_trim_downmix),
        "window.ui_customization" => flag(&mut app.extra.show_ui_customization),
        "window.config_auto_update" => flag(&mut app.extra.auto_update_config),
        _ => return None,
    })
}

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    auto_update(app, ctx);
    renderer(app, ctx);
    trim_downmix(app, ctx);
    ui_customization(app, ctx);
    apply_customization(app, ctx);
}

fn apply_customization(app: &mut SoundApp, ctx: &egui::Context) {
    let want = (app.extra.ui_scale.clamp(0.75, 1.5), app.extra.reduce_motion);
    if app.extra.applied == Some(want) || ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    if app.extra.applied.is_some() || (want.0 - 1.0).abs() > f32::EPSILON {
        ctx.set_zoom_factor(want.0);
    }
    ctx.global_style_mut(|s| s.animation_time = if want.1 { 0.0 } else { 1.0 / 12.0 });
    app.extra.applied = Some(want);
}

fn auto_update(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.extra.auto_update_config || app.ui.configurations.is_empty() {
        return;
    }
    let now = ctx.input(|i| i.time);
    if now - app.extra.last_auto_update >= AUTO_UPDATE_SECS {
        app.extra.last_auto_update = now;
        let _ = crate::menus::run_ui_command(app, "window.config_update", &json!({}));
    }
}

fn renderer(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.extra.show_renderer;
    if !open {
        return;
    }
    let t = Tokens::current();
    let fmt = app.engine.session().main_format();
    let peaks: Vec<f32> = app.player.as_ref().map(|p| p.meters().main.peaks).unwrap_or_default();
    let objects: Vec<(String, f32, f32, f32)> = app
        .engine
        .session()
        .tracks
        .iter()
        .filter(|tr| tr.kind != TrackKind::Master && app.engine.session().edit.flag(&format!("object.{}", tr.id.0)))
        .map(|tr| {
            let sp = openpt_mix::surround_at(tr, app.position()).unwrap_or_default();
            (tr.name.clone(), sp.x, sp.y, sp.z)
        })
        .collect();
    egui::Window::new(egui::RichText::new("Renderer").font(bold(13.0))).open(&mut open).default_size(vec2(380.0, 440.0)).resizable(true).fit_screen(ctx).show(
        ctx,
        |ui| {
            ui.label(egui::RichText::new(format!("Main output: {}", fmt.label())).font(regular(11.0)).color(t.text));
            let side = ui.available_width().min(360.0);
            let (r, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
            let p = ui.painter_at(r);
            p.rect_filled(r, 6.0, Color32::from_rgb(16, 19, 23));
            let c = r.center();
            let rad = side * 0.40;
            p.circle_stroke(c, rad, Stroke::new(1.0, Color32::from_rgb(60, 68, 78)));
            p.circle_stroke(c, rad * 0.55, Stroke::new(1.0, Color32::from_rgb(44, 50, 58)));
            p.text(pos2(c.x, r.min.y + 8.0), Align2::CENTER_TOP, "front", regular(9.0), t.text_dim);
            for (i, sp) in fmt.speakers().iter().enumerate() {
                let level = peaks.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                let ring = if sp.is_height() { rad * 0.55 } else { rad };
                let a = sp.azimuth().to_radians();
                let pos = if *sp == Speaker::Lfe { pos2(c.x, c.y + rad * 0.2) } else { pos2(c.x + ring * a.sin(), c.y - ring * a.cos()) };
                let glow = Color32::from_rgb(40 + (level * 60.0) as u8, 90 + (level * 140.0) as u8, 70 + (level * 40.0) as u8);
                p.circle_filled(pos, 9.0 + level * 6.0, glow);
                p.circle_stroke(pos, 9.0, Stroke::new(1.0, Color32::from_rgb(150, 160, 170)));
                p.text(pos, Align2::CENTER_CENTER, format!("{sp:?}"), bold(8.0), Color32::WHITE);
            }
            for (name, x, y, z) in &objects {
                let pos = pos2(c.x + x.clamp(-1.0, 1.0) * rad, c.y - y.clamp(-1.0, 1.0) * rad);
                p.circle_filled(pos, 5.0 + z.clamp(0.0, 1.0) * 3.0, Color32::from_rgb(240, 180, 60));
                p.text(pos2(pos.x + 8.0, pos.y), Align2::LEFT_CENTER, name, regular(9.0), Color32::from_rgb(240, 200, 120));
            }
            ui.add_space(4.0);
            if objects.is_empty() {
                ui.label(
                    egui::RichText::new("No object tracks. Use the Object view in the Mix window to route tracks as objects.")
                        .small()
                        .color(t.text_dim),
                );
            } else {
                ui.label(egui::RichText::new(format!("{} object track(s)", objects.len())).small().color(t.text_dim));
            }
            ui.separator();
            monitor_controls(app, ui);
            ui.horizontal(|ui| {
                if ui.button("Trim and Downmix…").clicked() {
                    app.extra.show_trim_downmix = true;
                }
                if ui.button("Export ADM BWF…").on_hover_text("Bed (main output) + object tracks as a Dolby Atmos ADM BWF master").clicked() {
                    export_adm(app);
                }
            });
        },
    );
    app.extra.show_renderer = open;
}

fn ui_customization(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.extra.show_ui_customization;
    if !open {
        return;
    }
    egui::Window::new(egui::RichText::new("Appearance").font(bold(13.0))).open(&mut open).default_size(vec2(320.0, 180.0)).resizable(false).fit_screen(ctx).show(
        ctx,
        |ui| {
            egui::Grid::new("ui_custom").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                ui.label("Interface scale");
                ui.add(egui::Slider::new(&mut app.extra.ui_scale, 0.75..=1.5).step_by(0.05).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)));
                ui.end_row();
                ui.label("Reduce motion");
                crate::widgets::checkbox(ui, &mut app.extra.reduce_motion, "");
                ui.end_row();
                ui.label("Theme");
                let mut chosen = None;
                egui::ComboBox::from_id_salt("theme_mode").selected_text(app.ui.theme.label()).show_ui(ui, |ui| {
                    for mode in crate::theme::ThemeMode::ALL {
                        if ui.selectable_label(app.ui.theme == mode, mode.label()).clicked() {
                            chosen = Some(mode);
                        }
                    }
                });
                if let Some(mode) = chosen {
                    let _ = app.run("ui.theme", json!({"mode": mode.id()}));
                }
                ui.end_row();
                ui.label("Auto-update configuration");
                crate::widgets::checkbox(ui, &mut app.extra.auto_update_config, "");
                ui.end_row();
            });
            if ui.button("Reset").clicked() {
                app.extra.ui_scale = 1.0;
                app.extra.reduce_motion = false;
                let _ = app.run("ui.theme", json!({"mode": crate::theme::ThemeMode::default().id()}));
            }
        },
    );
    app.extra.show_ui_customization = open;
}

fn set_atmos(app: &mut SoundApp, p: Value) {
    if let Err(e) = app.run("setup.atmos", p) {
        app.ui.status = e;
    }
}

fn monitor_controls(app: &mut SoundApp, ui: &mut egui::Ui) {
    let a = app.engine.session().atmos;
    let fmt = app.engine.session().main_format();
    ui.horizontal(|ui| {
        ui.label("Monitor");
        egui::ComboBox::from_id_salt("atmos_monitor").width(220.0).selected_text(a.monitor.label()).show_ui(ui, |ui| {
            for m in openpt_model::AtmosMonitor::ALL {
                if ui.selectable_label(a.monitor == m, m.label()).clicked() {
                    set_atmos(app, json!({"monitor": m.id()}));
                }
            }
        });
    });
    if fmt.channels() <= 2 && a.monitor != openpt_model::AtmosMonitor::Speakers {
        ui.weak("The main output is stereo: set an immersive format (e.g. 7.1.4) in I/O Setup to use the renderer.");
    }
    if a.monitor == openpt_model::AtmosMonitor::Binaural {
        let mut g = a.binaural_gain_db;
        if ui.add(egui::Slider::new(&mut g, -24.0..=12.0).text("Headphone level").suffix(" dB")).changed() {
            set_atmos(app, json!({"binaural_gain_db": g}));
        }
        binaural_controls(app, ui, a, fmt);
    }
}

fn distance_menu(ui: &mut egui::Ui, id: String, shown: &str) -> Option<openpt_model::BinauralDistance> {
    let mut chosen = None;
    egui::ComboBox::from_id_salt(id).width(70.0).selected_text(shown).show_ui(ui, |ui| {
        for d in openpt_model::BinauralDistance::ALL {
            if ui.selectable_label(shown == d.label(), d.label()).clicked() {
                chosen = Some(d);
            }
        }
    });
    chosen
}

fn binaural_controls(app: &mut SoundApp, ui: &mut egui::Ui, a: openpt_model::AtmosSettings, fmt: openpt_model::ChannelFormat) {
    let speakers = fmt.speakers();
    let used: Vec<_> = speakers.iter().enumerate().filter(|(_, s)| !s.is_lfe()).filter_map(|(c, _)| a.binaural_distance.get(c).copied()).collect();
    let shown = match used.first() {
        Some(first) if used.iter().all(|d| d == first) => first.label(),
        _ => "Mixed",
    };
    ui.horizontal(|ui| {
        ui.label("Bed distance");
        if let Some(d) = distance_menu(ui, "binaural_distance_all".to_string(), shown) {
            set_atmos(app, json!({"binaural_distance": d.id()}));
        }
    });
    let session = app.engine.session();
    let objects: Vec<(u64, String, openpt_model::BinauralDistance, bool)> = session
        .tracks
        .iter()
        .filter(|tr| tr.kind != TrackKind::Master && session.edit.is_object(tr.id))
        .map(|tr| (tr.id.0, tr.name.clone(), tr.mixer.binaural_distance, tr.mixer.output == openpt_model::Route::Main))
        .collect();
    if !objects.is_empty() {
        egui::CollapsingHeader::new("Distance per object").id_salt("binaural_distance_objects").default_open(true).show(ui, |ui| {
            egui::Grid::new("binaural_distance_objects_grid").num_columns(2).spacing(vec2(6.0, 3.0)).show(ui, |ui| {
                for (id, name, current, _) in &objects {
                    ui.label(name);
                    if let Some(d) = distance_menu(ui, format!("binaural_distance_object_{id}"), current.label()) {
                        let _ = app.run("track.binaural_distance", json!({"tracks": [id], "distance": d.id()}));
                    }
                    ui.end_row();
                }
            });
            if objects.iter().any(|(_, _, _, direct)| !direct) {
                ui.weak("Objects routed through a bus are heard at the bed distance.");
            }
        });
    }
    ui.weak("Saved in ADM masters as metadata. The room is never in the exported audio.");
    egui::CollapsingHeader::new("Distance per bed channel").id_salt("binaural_distance_channels").show(ui, |ui| {
        egui::Grid::new("binaural_distance_grid").num_columns(4).spacing(vec2(6.0, 3.0)).show(ui, |ui| {
            let mut column = 0;
            for (c, sp) in speakers.iter().enumerate() {
                if sp.is_lfe() {
                    continue;
                }
                let current = a.binaural_distance.get(c).copied().unwrap_or_default();
                ui.label(sp.label());
                if let Some(d) = distance_menu(ui, format!("binaural_distance_{c}"), current.label()) {
                    set_atmos(app, json!({"binaural_distance": d.id(), "channel": c}));
                }
                column += 1;
                if column % 2 == 0 {
                    ui.end_row();
                }
            }
        });
    });
    let profile = openpt_mix::hrtf::current().name.clone();
    ui.horizontal(|ui| {
        ui.label("Headphone profile");
        ui.label(egui::RichText::new(profile).color(Tokens::current().text));
        if ui.button("Load SOFA…").on_hover_text("Use your own HRTF measurement (AES69 .sofa file)").clicked()
            && let Some(pick) = &app.services.pick_open
            && let Some(path) = pick("renderer.hrtf", &["sofa"])
        {
            match openpt_mix::hrtf::load_sofa(std::path::Path::new(&path)) {
                Ok(name) => {
                    app.ui.hrtf_sofa = path;
                    app.ui.status = format!("Headphone profile: {name}");
                }
                Err(e) => app.ui.status = format!("Could not read this SOFA file: {e}"),
            }
        }
        if !app.ui.hrtf_sofa.is_empty() && ui.button("Use KEMAR").clicked() {
            openpt_mix::hrtf::use_kemar();
            app.ui.hrtf_sofa.clear();
        }
    });
}

fn export_adm(app: &mut SoundApp) {
    let s = app.engine.session();
    let suggested = format!("{} Atmos.wav", s.name);
    let dir = app.engine.path.as_deref().and_then(|p| std::path::Path::new(p).parent()).map(std::path::Path::to_path_buf);
    let start = dir.map_or_else(|| suggested.clone(), |d| d.join(&suggested).display().to_string());
    let picked = match &app.services.pick_save {
        Some(pick) => pick("file.export_adm", &start),
        None => Some(start),
    };
    let Some(path) = picked else { return };
    let sel = app.engine.session().edit.selection;
    let mut p = json!({"path": path});
    if !sel.is_empty() {
        p["start"] = json!(sel.start);
        p["end"] = json!(sel.end);
    }
    app.ui.status = match app.run("file.export_adm", p) {
        Ok(v) => {
            let mut msg = format!("ADM BWF written: {} bed + {} object(s), {} channels", v["bed"].as_str().unwrap_or(""), v["objects"], v["channels"]);
            if let Some(w) = v["warning"].as_str() {
                msg.push_str(" · ");
                msg.push_str(w);
            }
            msg
        }
        Err(e) => e,
    };
}

fn trim_downmix(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.extra.show_trim_downmix;
    if !open {
        return;
    }
    egui::Window::new(egui::RichText::new("Trim and Downmix").font(bold(13.0))).open(&mut open).resizable(false).fit_screen(ctx).show(ctx, |ui| {
        let a = app.engine.session().atmos;
        ui.label(format!("Source: {}", app.engine.session().main_format().label()));
        monitor_controls(app, ui);
        ui.separator();
        ui.strong("Downmix trims");
        let (mut s, mut h) = (a.surround_trim_db, a.height_trim_db);
        let rs = ui.add(egui::Slider::new(&mut s, -24.0..=6.0).text("Surround trim").suffix(" dB"));
        let rh = ui.add(egui::Slider::new(&mut h, -24.0..=6.0).text("Height trim").suffix(" dB"));
        if rs.changed() || rh.changed() {
            set_atmos(app, json!({"surround_trim_db": s, "height_trim_db": h}));
        }
        ui.horizontal(|ui| {
            for (label, sv, hv) in [("Music (−3 / −3)", -3.0, -3.0), ("Film (−3 / −6)", -3.0, -6.0), ("Flat (0 / 0)", 0.0, 0.0)] {
                if ui.small_button(label).clicked() {
                    set_atmos(app, json!({"surround_trim_db": sv, "height_trim_db": hv}));
                }
            }
        });
        ui.weak("Trims shape the 5.1 and stereo (Lo/Ro) downmixes you monitor; the bed and objects themselves are not changed.");
    });
    app.extra.show_trim_downmix = open;
}
