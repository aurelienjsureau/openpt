use crate::SoundApp;
use crate::theme::{Tokens, bold};
use egui::{Align2, vec2};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
enum Tab {
    Create,
    Recent,
}

#[derive(Debug, Clone)]
struct Draft {
    tab: Tab,
    name: String,
    demo: bool,
    sample_rate: u32,
    bit_depth: String,
    dir: String,
    picked: Option<usize>,
    error: String,
}

fn draft_id() -> egui::Id {
    egui::Id::new("dashboard_draft")
}

fn file_name(name: &str) -> String {
    let n: String = name.trim().chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    if n.is_empty() { "Untitled".into() } else { n }
}

pub fn session_path(dir: &str, name: &str) -> String {
    let n = file_name(name);
    Path::new(dir).join(&n).join(format!("{n}.openpt")).to_string_lossy().into_owned()
}

fn create_params(d: &Draft) -> Value {
    let mut p = json!({
        "name": d.name.trim(),
        "sample_rate": d.sample_rate,
        "bit_depth": d.bit_depth,
        "template": if d.demo { "demo" } else { "blank" },
    });
    if !d.dir.trim().is_empty() {
        p["path"] = json!(session_path(d.dir.trim(), &d.name));
    }
    p
}

fn default_dir(app: &SoundApp) -> String {
    if !app.ui.session_dir.is_empty() {
        return app.ui.session_dir.clone();
    }
    let home = crate::dialogs::default_dir();
    if home.is_empty() { home } else { Path::new(&home).join("Documents").join("OpenPT").to_string_lossy().into_owned() }
}

pub fn open(app: &mut SoundApp, ctx: &egui::Context) {
    app.ui.show_dashboard = true;
    ctx.data_mut(|d| d.remove::<Draft>(draft_id()));
}

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.ui.show_dashboard {
        return;
    }
    let mut d = ctx.data(|d| d.get_temp::<Draft>(draft_id())).unwrap_or_else(|| Draft {
        tab: if app.engine.path.is_none() && !app.ui.recent_sessions.is_empty() && app.ui.dashboard_tab_recent { Tab::Recent } else { Tab::Create },
        name: "Untitled".into(),
        demo: false,
        sample_rate: 48_000,
        bit_depth: "24".into(),
        dir: default_dir(app),
        picked: None,
        error: String::new(),
    });
    let mut close = false;
    let mut run: Option<(&'static str, Value)> = None;
    let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    let now = ctx.input(|i| i.time);
    let exists = app.recent_exists(now);
    egui::Window::new(egui::RichText::new("Dashboard").font(bold(13.0)))
        .collapsible(false)
        .resizable(false)
        .fixed_size(vec2(620.0, 360.0))
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -40.0))
        .show(ctx, |ui| {
            let t = Tokens::current();
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(110.0);
                    ui.add_space(4.0);
                    for (tab, label) in [(Tab::Create, "Create"), (Tab::Recent, "Recent")] {
                        if ui.add(egui::Button::selectable(d.tab == tab, label).min_size(vec2(110.0, 28.0))).clicked() {
                            d.tab = tab;
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_min_height(290.0);
                    match d.tab {
                        Tab::Create => {
                            egui::Grid::new("dash_create").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                                ui.label("Name");
                                ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(320.0));
                                ui.end_row();
                                ui.label("Template");
                                egui::ComboBox::from_id_salt("dash_tpl").width(320.0).selected_text(if d.demo { "Demo session" } else { "Blank" }).show_ui(
                                    ui,
                                    |ui| {
                                        ui.selectable_value(&mut d.demo, false, "Blank");
                                        ui.selectable_value(&mut d.demo, true, "Demo session");
                                    },
                                );
                                ui.end_row();
                                ui.label("Sample Rate");
                                egui::ComboBox::from_id_salt("dash_rate").width(320.0).selected_text(format!("{} kHz", f64::from(d.sample_rate) / 1000.0)).show_ui(
                                    ui,
                                    |ui| {
                                        for r in openpt_time::SampleRate::COMMON {
                                            ui.selectable_value(&mut d.sample_rate, r, format!("{} kHz", f64::from(r) / 1000.0));
                                        }
                                    },
                                );
                                ui.end_row();
                                ui.label("Bit Depth");
                                egui::ComboBox::from_id_salt("dash_bits").width(320.0).selected_text(bits_label(&d.bit_depth)).show_ui(ui, |ui| {
                                    for b in ["16", "24", "32f"] {
                                        ui.selectable_value(&mut d.bit_depth, b.to_string(), bits_label(b));
                                    }
                                });
                                ui.end_row();
                                ui.label("Location");
                                ui.horizontal(|ui| {
                                    ui.add(egui::TextEdit::singleline(&mut d.dir).desired_width(240.0));
                                    if let Some(pick) = &app.services.pick_folder
                                        && ui.button("Browse…").clicked()
                                        && let Some(p) = pick(&d.dir)
                                    {
                                        d.dir = p;
                                    }
                                });
                                ui.end_row();
                            });
                            ui.add_space(6.0);
                            let note = if d.dir.trim().is_empty() {
                                "The session is created untitled; Save chooses where it goes.".to_string()
                            } else {
                                format!("Creates {}", session_path(d.dir.trim(), &d.name))
                            };
                            ui.label(egui::RichText::new(note).small().color(t.text_dim));
                        }
                        Tab::Recent => {
                            if app.ui.recent_sessions.is_empty() {
                                ui.label(egui::RichText::new("No recent sessions.").color(t.text_dim));
                            }
                            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                                for (i, path) in app.ui.recent_sessions.iter().enumerate() {
                                    let here = exists.get(i).copied().unwrap_or(true);
                                    let p = Path::new(path);
                                    let name = p.file_stem().and_then(|n| n.to_str()).unwrap_or(path);
                                    let text = if here { name.to_string() } else { format!("{name}  (missing)") };
                                    let r = ui
                                        .add_enabled(here, egui::Button::selectable(d.picked == Some(i), text).min_size(vec2(460.0, 24.0)))
                                        .on_hover_text(path.as_str());
                                    if r.clicked() {
                                        d.picked = Some(i);
                                    }
                                    if r.double_clicked() {
                                        run = Some(("session.open", json!({"path": path})));
                                    }
                                }
                            });
                        }
                    }
                    if !d.error.is_empty() {
                        ui.colored_label(t.rec, d.error.as_str());
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                crate::widgets::checkbox(ui, &mut app.ui.dashboard_at_startup, "Show on startup");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ok = if d.tab == Tab::Create { "Create" } else { "Open" };
                    let enabled = d.tab == Tab::Create || d.picked.is_some();
                    if ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(ok).strong()).fill(t.accent)).clicked() || (enter && enabled) {
                        run = match d.tab {
                            Tab::Create => Some(("session.new", create_params(&d))),
                            Tab::Recent => d.picked.and_then(|i| app.ui.recent_sessions.get(i)).map(|p| ("session.open", json!({"path": p}))),
                        };
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if let Some(pick) = &app.services.pick_open
                        && ui.button("Open from Disk…").clicked()
                        && let Some(p) = pick("session.open", &[])
                    {
                        run = Some(("session.open", json!({"path": p})));
                    }
                });
            });
        });
    if let Some((cmd, params)) = run {
        match crate::dialogs::run_or_ask(app, cmd, params) {
            Ok(_) => {
                if cmd == "session.new" && !d.dir.trim().is_empty() {
                    app.ui.session_dir = d.dir.trim().to_string();
                }
                app.ui.dashboard_tab_recent = d.tab == Tab::Recent;
                close = true;
            }
            Err(e) => d.error = e,
        }
    }
    if close {
        app.ui.show_dashboard = false;
        ctx.data_mut(|m| m.remove::<Draft>(draft_id()));
    } else {
        ctx.data_mut(|m| m.insert_temp(draft_id(), d));
    }
}

fn bits_label(b: &str) -> &'static str {
    match b {
        "16" => "16 Bit",
        "32f" => "32 Bit Float",
        _ => "24 Bit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sessions_get_their_own_folder_and_a_safe_name() {
        let p = session_path("/music", "Song: take 2");
        assert_eq!(Path::new(&p), Path::new("/music/Song_ take 2/Song_ take 2.openpt"));
        assert!(session_path("/music", "  ").ends_with("Untitled.openpt"));
    }

    #[test]
    fn create_runs_session_new_and_saves_it_where_asked() {
        let dir = std::env::temp_dir().join(format!("openpt-dashboard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let d = Draft {
            tab: Tab::Create,
            name: "Mix".into(),
            demo: false,
            sample_rate: 96_000,
            bit_depth: "32f".into(),
            dir: dir.to_string_lossy().into_owned(),
            picked: None,
            error: String::new(),
        };
        let mut e = openpt_engine::Engine::default();
        let out = e.execute("session.new", &create_params(&d)).unwrap();
        assert_eq!(out["sample_rate"], 96_000);
        assert!(dir.join("Mix").join("Mix.openpt").is_file());
        assert_eq!(e.session().bit_depth, openpt_model::BitDepthSetting::Float32);
        let again = e.execute("session.new", &create_params(&d)).unwrap_err().to_string();
        assert!(again.contains("already exists"), "{again}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
