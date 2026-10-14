use crate::SoundApp;
use crate::theme::{Tokens, bold, regular};
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use openpt_dsp::{PluginInfo, Unit};
use openpt_model::{Insert, TrackId};
use serde_json::json;

const PANEL: Color32 = Color32::from_rgb(44, 46, 49);
const PANEL_EDGE: Color32 = Color32::from_rgb(24, 25, 27);
const LABEL: Color32 = Color32::from_rgb(196, 200, 204);
const READOUT_BG: Color32 = Color32::from_rgb(10, 12, 12);
const CELL: egui::Vec2 = egui::Vec2::new(78.0, 96.0);

pub fn header(
    app: &mut SoundApp,
    ui: &mut Ui,
    tid: TrackId,
    slot: usize,
    info: &PluginInfo,
    ins: &Insert,
    presets: impl FnOnce(&mut SoundApp, &mut Ui),
) {
    let t = Tokens::current();
    let small = |s: &str| egui::RichText::new(s).font(regular(9.5)).color(t.text_dim);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(small("Track"));
            let name = app.engine.session().track(tid).map(|t| t.name.clone()).unwrap_or_default();
            crate::widgets::value_box(ui, |ui| ui.add_sized([120.0, 18.0], egui::Label::new(egui::RichText::new(name).font(bold(11.0)))));
        });
        ui.vertical(|ui| {
            ui.label(small("Preset"));
            ui.horizontal(|ui| {
                ui.menu_button(
                    egui::RichText::new(if ins.preset.is_empty() { "<factory default>" } else { ins.preset.as_str() }).font(regular(11.0)),
                    |ui| presets(app, ui),
                );
                ui.add_enabled(false, egui::Button::new("−").small());
                ui.add_enabled(false, egui::Button::new("+").small());
                ui.add_enabled(false, egui::Button::new("Compare").small());
            });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
            let bypass = ui.add(
                egui::Button::new(egui::RichText::new("⏻").font(bold(13.0)).color(if ins.bypass { Color32::WHITE } else { t.text_dim }))
                    .fill(if ins.bypass { Color32::from_rgb(40, 110, 200) } else { t.slot_face[0] })
                    .min_size(vec2(26.0, 20.0)),
            );
            if bypass.on_hover_text("Bypass").clicked() {
                let _ = app.run("mix.insert_bypass", json!({"track": tid.0, "slot": slot}));
            }
            ui.add_enabled(false, egui::Button::new("Auto").small());
            ui.label(small(&format!("{} · {}", char::from(b'a'.saturating_add(slot as u8)), info.name)));
        });
    });
}

pub fn panel(ui: &mut Ui, info: &PluginInfo, ins: &Insert, mut set: impl FnMut(&str, f32)) {
    let cols = info.params.len().clamp(1, 6);
    let rows = info.params.len().div_ceil(cols);
    let graph = graph_kind(info).is_some();
    let width = cols as f32 * CELL.x + 24.0;
    let height = 34.0 + if graph { 130.0 } else { 0.0 } + rows as f32 * CELL.y + 10.0;
    let (r, _) = ui.allocate_exact_size(vec2(width.max(360.0), height), Sense::hover());
    let p = ui.painter();
    p.rect(r, CornerRadius::same(3), PANEL, Stroke::new(1.0, PANEL_EDGE), StrokeKind::Inside);
    p.text(pos2(r.min.x + 12.0, r.min.y + 16.0), Align2::LEFT_CENTER, info.name.to_uppercase(), bold(13.0), Color32::from_rgb(232, 234, 236));
    p.text(pos2(r.max.x - 12.0, r.min.y + 16.0), Align2::RIGHT_CENTER, "OpenPT", regular(9.5), LABEL);
    let mut y = r.min.y + 34.0;
    if let Some(kind) = graph_kind(info) {
        let g = Rect::from_min_size(pos2(r.min.x + 12.0, y), vec2(r.width() - 24.0, 120.0));
        draw_graph(p, g, kind, info, ins);
        y += 130.0;
    }
    let left = r.min.x + (r.width() - cols as f32 * CELL.x) / 2.0;
    for (i, prm) in info.params.iter().enumerate() {
        let cell = Rect::from_min_size(pos2(left + (i % cols) as f32 * CELL.x, y + (i / cols) as f32 * CELL.y), CELL);
        let v = ins.params.get(prm.id).copied().unwrap_or(prm.default);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(cell).layout(egui::Layout::top_down(egui::Align::Center)));
        child.label(egui::RichText::new(prm.name.to_uppercase()).font(regular(9.0)).color(LABEL));
        match prm.unit {
            Unit::Toggle => {
                let on = v >= 0.5;
                child.add_space(8.0);
                let b = egui::Button::new(egui::RichText::new(if on { "ON" } else { "OFF" }).font(bold(10.0)).color(if on {
                    Color32::BLACK
                } else {
                    LABEL
                }))
                .fill(if on { Color32::from_rgb(120, 200, 110) } else { Color32::from_rgb(58, 60, 64) })
                .min_size(vec2(44.0, 22.0));
                if child.add(b).clicked() {
                    set(prm.id, if on { 0.0 } else { 1.0 });
                }
            }
            Unit::Choice => {
                child.add_space(8.0);
                let mut idx = v.round().max(0.0) as usize;
                egui::ComboBox::from_id_salt(("plugin_choice", prm.id))
                    .width(CELL.x - 10.0)
                    .selected_text(prm.choices.get(idx).copied().unwrap_or("?"))
                    .show_ui(&mut child, |ui| {
                        for (k, c) in prm.choices.iter().enumerate() {
                            ui.selectable_value(&mut idx, k, *c);
                        }
                    });
                if idx as f32 != v.round() {
                    set(prm.id, idx as f32);
                }
            }
            _ => {
                let mut n = prm.to_normalized(v);
                let def = prm.to_normalized(prm.default);
                let resp = crate::widgets::knob(&mut child, 44.0, &mut n, 0.0..=1.0, def, prm.name);
                if resp.clicked() && crate::mix_window::click_mods(&child).alt {
                    n = def;
                }
                if (prm.from_normalized(n) - v).abs() > f32::EPSILON {
                    set(prm.id, prm.from_normalized(n));
                }
            }
        }
        let rr = Rect::from_center_size(pos2(cell.center().x, cell.max.y - 14.0), vec2(CELL.x - 12.0, 15.0));
        ui.painter().rect_filled(rr, 2.0, READOUT_BG);
        ui.painter().text(rr.center(), Align2::CENTER_CENTER, prm.format(v), regular(10.0), Tokens::DARK.counter_text);
    }
}

#[derive(Clone, Copy)]
enum Graph {
    Eq7,
    Eq1,
    Dynamics,
}

fn graph_kind(info: &PluginInfo) -> Option<Graph> {
    match info.id {
        "eq_7band" => Some(Graph::Eq7),
        "eq_1band" => Some(Graph::Eq1),
        _ if info.params.iter().any(|p| p.id == "threshold") => Some(Graph::Dynamics),
        _ => None,
    }
}

fn draw_graph(p: &egui::Painter, g: Rect, kind: Graph, info: &PluginInfo, ins: &Insert) {
    let curve = Color32::from_rgb(120, 190, 255);
    let grid = Color32::from_rgb(38, 48, 58);
    p.rect(g, CornerRadius::same(2), Color32::from_rgb(12, 16, 20), Stroke::new(1.0, PANEL_EDGE), StrokeKind::Inside);
    let val = |id: &str| ins.params.get(id).copied().or_else(|| info.params.iter().find(|q| q.id == id).map(|q| q.default)).unwrap_or(0.0);
    match kind {
        Graph::Eq7 | Graph::Eq1 => {
            for f in [100.0f32, 1000.0, 10_000.0] {
                let x = g.min.x + g.width() * (f / 20.0).log10() / 3.0;
                p.vline(x, g.y_range(), Stroke::new(1.0, grid));
                p.text(
                    pos2(x + 3.0, g.max.y - 7.0),
                    Align2::LEFT_CENTER,
                    if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") },
                    regular(8.5),
                    LABEL,
                );
            }
            for db in [-12.0f32, 0.0, 12.0] {
                let y = g.center().y - db / 18.0 * g.height() * 0.5;
                p.hline(g.x_range(), y, Stroke::new(1.0, grid));
                p.text(pos2(g.min.x + 3.0, y - 6.0), Align2::LEFT_CENTER, format!("{db:+}"), regular(8.5), LABEL);
            }
            let n = 200;
            let freqs: Vec<f32> = (0..n).map(|i| 20.0 * 1000f32.powf(i as f32 / (n - 1) as f32)).collect();
            let params: Vec<(&str, f32)> = ins.params.iter().map(|(k, v)| (k.as_str(), *v)).collect();
            let resp = if matches!(kind, Graph::Eq7) {
                openpt_dsp::eq7_response(&params, &freqs, 48_000.0)
            } else {
                openpt_dsp::eq1_response(&params, &freqs, 48_000.0)
            };
            let pts: Vec<egui::Pos2> = resp
                .iter()
                .enumerate()
                .map(|(i, db)| pos2(g.min.x + g.width() * i as f32 / (n - 1) as f32, g.center().y - db.clamp(-18.0, 18.0) / 18.0 * g.height() * 0.5))
                .collect();
            p.add(egui::Shape::line(pts, Stroke::new(2.0, curve)));
        }
        Graph::Dynamics => {
            let side = g.height();
            let sq = Rect::from_min_size(pos2(g.center().x - side / 2.0, g.min.y), vec2(side, side));
            for db in [-48.0f32, -36.0, -24.0, -12.0] {
                let k = 1.0 + db / 60.0;
                p.vline(sq.min.x + sq.width() * k, sq.y_range(), Stroke::new(1.0, grid));
                p.hline(sq.x_range(), sq.max.y - sq.height() * k, Stroke::new(1.0, grid));
            }
            let (th, ratio, knee) = (val("threshold"), val("ratio").max(1.0), val("knee").max(0.0));
            let gate = info.id.contains("gate") || info.id.contains("expander");
            let out = |x: f32| -> f32 {
                let over = x - th;
                if gate {
                    if over >= 0.0 { x } else { th + over * ratio }
                } else if knee > 0.0 && over.abs() < knee / 2.0 {
                    x + (1.0 / ratio - 1.0) * (over + knee / 2.0).powi(2) / (2.0 * knee)
                } else if over > 0.0 {
                    th + over / ratio
                } else {
                    x
                }
            };
            let n = 120;
            let pts: Vec<egui::Pos2> = (0..n)
                .map(|i| {
                    let x = -60.0 + 60.0 * i as f32 / (n - 1) as f32;
                    let y = out(x).clamp(-60.0, 0.0);
                    pos2(sq.min.x + sq.width() * (x + 60.0) / 60.0, sq.max.y - sq.height() * (y + 60.0) / 60.0)
                })
                .collect();
            p.add(egui::Shape::line(pts, Stroke::new(2.0, curve)));
            p.rect_stroke(sq, 0.0, Stroke::new(1.0, grid), StrokeKind::Inside);
            p.text(pos2(sq.max.x + 6.0, sq.min.y + 8.0), Align2::LEFT_CENTER, format!("Threshold {th:.1} dB"), regular(9.0), LABEL);
            p.text(pos2(sq.max.x + 6.0, sq.min.y + 22.0), Align2::LEFT_CENTER, format!("Ratio {ratio:.1}:1"), regular(9.0), LABEL);
        }
    }
}
