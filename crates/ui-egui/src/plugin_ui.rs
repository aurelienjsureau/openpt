use crate::SoundApp;
use crate::theme::{Tokens, bold, regular};
use crate::widgets::FitScreen;
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
    _info: &PluginInfo,
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
                ui.add_enabled(false, egui::Button::new(t!("Compare")).small());
            });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
            let bypass = ui.add(
                egui::Button::new(egui::RichText::new("⏻").font(bold(13.0)).color(if ins.bypass { Color32::WHITE } else { t.text_dim }))
                    .fill(if ins.bypass { Color32::from_rgb(40, 110, 200) } else { t.slot_face[0] })
                    .min_size(vec2(26.0, 20.0)),
            );
            if bypass.on_hover_text(t!("Bypass")).clicked() {
                let _ = app.run("mix.insert_bypass", json!({"track": tid.0, "slot": slot}));
            }
            if ui.add(egui::Button::new(t!("Auto")).small()).on_hover_text(t!("Plug-in Automation: choose what can be automated")).clicked() {
                open_automation_dialog(ui.ctx(), tid, slot);
            }
        });
    });
    if takes_key(&ins.plugin) {
        ui.horizontal(|ui| key_input(app, ui, tid, slot, ins));
    }
}

pub fn panel(ui: &mut Ui, info: &PluginInfo, ins: &Insert, gr: f32, mut set: impl FnMut(&str, f32)) {
    match info.id {
        "eq_7band" => return eq7_panel(ui, info, ins, &mut set),
        "compressor" | "expander_gate" => return dynamics_panel(ui, info, ins, gr, &mut set),
        id => {
            if let Some((sections, meter)) = sections_of(id) {
                return sections_panel(ui, info, ins, gr, sections, meter, &mut set);
            }
        }
    }
    let cols = info.params.len().clamp(1, 6);
    let rows = info.params.len().div_ceil(cols);
    let graph = graph_kind(info).is_some();
    let width = cols as f32 * CELL.x + 24.0;
    let height = 34.0 + if graph { 130.0 } else { 0.0 } + rows as f32 * CELL.y + 10.0;
    let r = frame(ui, info, vec2(width.max(360.0), height));
    let mut y = r.min.y + 34.0;
    if let Some(kind) = graph_kind(info) {
        let g = Rect::from_min_size(pos2(r.min.x + 12.0, y), vec2(r.width() - 24.0, 120.0));
        draw_graph(ui.painter(), g, kind, info, ins);
        y += 130.0;
    }
    let left = r.min.x + (r.width() - cols as f32 * CELL.x) / 2.0;
    for (i, prm) in info.params.iter().enumerate() {
        let cell = Rect::from_min_size(pos2(left + (i % cols) as f32 * CELL.x, y + (i / cols) as f32 * CELL.y), CELL);
        param_cell(ui, cell, prm, ins, &mut set);
    }
}

fn frame(ui: &mut Ui, info: &PluginInfo, size: egui::Vec2) -> Rect {
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect(r, CornerRadius::same(3), PANEL, Stroke::new(1.0, PANEL_EDGE), StrokeKind::Inside);
    p.text(pos2(r.min.x + 12.0, r.min.y + 16.0), Align2::LEFT_CENTER, info.name.to_uppercase(), bold(13.0), Color32::from_rgb(232, 234, 236));
    p.text(pos2(r.max.x - 12.0, r.min.y + 16.0), Align2::RIGHT_CENTER, t!("OpenPT"), regular(9.5), LABEL);
    r
}

fn param_cell(ui: &mut Ui, cell: Rect, prm: &openpt_dsp::ParamInfo, ins: &Insert, set: &mut dyn FnMut(&str, f32)) {
    let v = ins.params.get(prm.id).copied().unwrap_or(prm.default);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(cell).layout(egui::Layout::top_down(egui::Align::Center)));
    child.label(egui::RichText::new(prm.name.to_uppercase()).font(regular(9.0)).color(LABEL));
    match prm.unit {
        Unit::Toggle => {
            let on = v >= 0.5;
            child.add_space(8.0);
            let b =
                egui::Button::new(egui::RichText::new(if on { "ON" } else { "OFF" }).font(bold(10.0)).color(if on { Color32::BLACK } else { LABEL }))
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
                .width(cell.width() - 10.0)
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
    let rr = Rect::from_center_size(pos2(cell.center().x, cell.max.y - 14.0), vec2(cell.width() - 12.0, 15.0));
    ui.painter().rect_filled(rr, 2.0, READOUT_BG);
    ui.painter().text(rr.center(), Align2::CENTER_CENTER, prm.format(v), regular(10.0), Tokens::DARK.counter_text);
}

fn eq7_panel(ui: &mut Ui, info: &PluginInfo, ins: &Insert, set: &mut dyn FnMut(&str, f32)) {
    let grey = Color32::from_rgb(150, 154, 160);
    let bands: [(&str, Color32, &[&str]); 9] = [
        ("INPUT", grey, &["input_gain"]),
        ("HPF", grey, &["hpf_on", "hpf_freq", "hpf_slope"]),
        ("LF", Color32::from_rgb(222, 96, 84), &["low_shelf_freq", "low_shelf_gain", "low_shelf_q"]),
        ("LMF", Color32::from_rgb(232, 168, 60), &["low_mid_freq", "low_mid_gain", "low_mid_q"]),
        ("MF", Color32::from_rgb(110, 196, 92), &["mid_freq", "mid_gain", "mid_q"]),
        ("HMF", Color32::from_rgb(76, 170, 222), &["high_mid_freq", "high_mid_gain", "high_mid_q"]),
        ("HF", Color32::from_rgb(170, 120, 230), &["high_shelf_freq", "high_shelf_gain", "high_shelf_q"]),
        ("LPF", grey, &["lpf_on", "lpf_freq", "lpf_slope"]),
        ("OUTPUT", grey, &["output_gain"]),
    ];
    let col_w = 78.0;
    let width = bands.len() as f32 * col_w + 24.0;
    let height = 34.0 + 170.0 + 22.0 + 3.0 * CELL.y + 10.0;
    let r = frame(ui, info, vec2(width, height));
    let g = Rect::from_min_size(pos2(r.min.x + 12.0, r.min.y + 34.0), vec2(r.width() - 24.0, 160.0));
    draw_graph(ui.painter(), g, Graph::Eq7, info, ins);
    let val = |id: &str| ins.params.get(id).copied().or_else(|| info.param(id).map(|p| p.default)).unwrap_or(0.0);
    let x_of = |f: f32| g.min.x + g.width() * (f.max(20.0) / 20.0).log10() / 3.0;
    let f_of = |x: f32| 20.0 * 10f32.powf(((x - g.min.x) / g.width()).clamp(0.0, 1.0) * 3.0);
    let y_of = |db: f32| g.center().y - db.clamp(-18.0, 18.0) / 18.0 * g.height() * 0.5;
    let db_of = |y: f32| ((g.center().y - y) / (g.height() * 0.5) * 18.0).clamp(-18.0, 18.0);
    for (title, col, ids) in bands.iter().skip(1).take(7) {
        let Some(freq_id) = ids.iter().find(|i| i.ends_with("_freq")).copied() else { continue };
        let gain_id = ids.iter().find(|i| i.ends_with("_gain")).copied();
        if ids.iter().find(|i| i.ends_with("_on")).is_some_and(|o| val(o) < 0.5) {
            continue;
        }
        let at = pos2(x_of(val(freq_id)), y_of(gain_id.map_or(0.0, val)));
        let resp = ui.interact(Rect::from_center_size(at, vec2(14.0, 14.0)), ui.id().with(("eq7_handle", *title)), Sense::drag());
        ui.painter().circle(at, if resp.hovered() || resp.dragged() { 6.5 } else { 5.0 }, *col, Stroke::new(1.0, Color32::BLACK));
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            if let Some(pi) = info.param(freq_id) {
                set(freq_id, f_of(p.x).clamp(pi.min, pi.max));
            }
            if let Some(gid) = gain_id
                && let Some(pi) = info.param(gid)
            {
                set(gid, db_of(p.y).clamp(pi.min, pi.max));
            }
        }
        resp.on_hover_text(format!("{title}: drag to set frequency and gain"));
    }
    let top = g.max.y + 10.0;
    let left = r.min.x + 12.0;
    for (k, (title, col, ids)) in bands.iter().enumerate() {
        let x = left + k as f32 * col_w;
        let head = Rect::from_min_size(pos2(x + 3.0, top), vec2(col_w - 6.0, 18.0));
        ui.painter().rect_filled(head, 2.0, col.gamma_multiply(0.35));
        ui.painter().text(head.center(), Align2::CENTER_CENTER, *title, bold(10.5), *col);
        for (j, pid) in ids.iter().enumerate() {
            let Some(prm) = info.param(pid) else { continue };
            let cell = Rect::from_min_size(pos2(x, top + 22.0 + j as f32 * CELL.y), vec2(col_w, CELL.y));
            param_cell(ui, cell, prm, ins, set);
        }
    }
}

fn dynamics_panel(ui: &mut Ui, info: &PluginInfo, ins: &Insert, gr: f32, set: &mut dyn FnMut(&str, f32)) {
    let main: &[&str] = if info.id == "compressor" {
        &["threshold", "ratio", "knee", "attack", "release", "makeup"]
    } else {
        &["threshold", "ratio", "range", "attack", "hold", "release"]
    };
    let rest: Vec<&openpt_dsp::ParamInfo> = info.params.iter().filter(|p| !main.contains(&p.id)).collect();
    let graph: f32 = 200.0;
    let upper = graph.max(2.0 * CELL.y);
    let width = (12.0 + graph + 50.0 + 3.0 * CELL.x + 12.0).max(5.0 * CELL.x + 24.0);
    let height = 34.0 + upper + 8.0 + rest.len().div_ceil(5) as f32 * CELL.y + 10.0;
    let r = frame(ui, info, vec2(width, height));
    let g = Rect::from_min_size(pos2(r.min.x + 12.0, r.min.y + 34.0), vec2(graph, graph));
    draw_graph(ui.painter(), g, Graph::Dynamics, info, ins);
    let m = Rect::from_min_size(pos2(g.max.x + 12.0, g.min.y + 14.0), vec2(12.0, graph - 14.0));
    let p = ui.painter();
    p.text(pos2(m.center().x, g.min.y + 5.0), Align2::CENTER_CENTER, t!("GR"), bold(9.0), LABEL);
    p.rect_filled(m, 2.0, READOUT_BG);
    let k = (gr.max(0.0) / 20.0).clamp(0.0, 1.0);
    if k > 0.0 {
        p.rect_filled(Rect::from_min_max(m.min, pos2(m.max.x, m.min.y + m.height() * k)), 2.0, Color32::from_rgb(232, 160, 40));
    }
    for db in [3.0f32, 6.0, 10.0, 15.0] {
        let y = m.min.y + m.height() * db / 20.0;
        p.hline(m.max.x + 1.0..=m.max.x + 4.0, y, Stroke::new(1.0, LABEL));
        p.text(pos2(m.max.x + 6.0, y), Align2::LEFT_CENTER, format!("{db}"), regular(8.0), LABEL);
    }
    let knobs_x = m.max.x + 26.0;
    for (i, pid) in main.iter().enumerate() {
        let Some(prm) = info.param(pid) else { continue };
        let cell = Rect::from_min_size(pos2(knobs_x + (i % 3) as f32 * CELL.x, g.min.y + (i / 3) as f32 * CELL.y), CELL);
        param_cell(ui, cell, prm, ins, set);
    }
    let y = r.min.y + 34.0 + upper + 8.0;
    for (i, prm) in rest.iter().enumerate() {
        let cell = Rect::from_min_size(pos2(r.min.x + 12.0 + (i % 5) as f32 * CELL.x, y + (i / 5) as f32 * CELL.y), CELL);
        param_cell(ui, cell, prm, ins, set);
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
            p.text(pos2(sq.min.x + 6.0, sq.min.y + 9.0), Align2::LEFT_CENTER, format!("Threshold {th:.1} dB"), regular(9.0), LABEL);
            p.text(pos2(sq.min.x + 6.0, sq.min.y + 23.0), Align2::LEFT_CENTER, format!("Ratio {ratio:.1}:1"), regular(9.0), LABEL);
        }
    }
}

pub fn takes_key(plugin: &str) -> bool {
    openpt_mix::is_third_party(plugin)
        || matches!(plugin, "compressor" | "expander_gate" | "de_esser" | "open_eq" | "open_mb" | "open_comp" | "open_g")
}

pub fn key_input(app: &mut SoundApp, ui: &mut Ui, tid: TrackId, slot: usize, ins: &Insert) {
    let t = Tokens::current();
    let busses: Vec<(u64, String)> = app.engine.session().busses.iter().map(|b| (b.id.0, b.name.clone())).collect();
    let current = ins.key.and_then(|k| busses.iter().find(|(id, _)| *id == k.0)).map_or("no key input", |(_, n)| n.as_str()).to_string();
    let lit = |on: bool| if on { Color32::from_rgb(232, 160, 40) } else { t.slot_face[0] };
    let (r, resp) = ui.allocate_exact_size(vec2(16.0, 14.0), Sense::hover());
    let col = if ins.key_on { Color32::from_rgb(232, 160, 40) } else { t.text_dim };
    let p = ui.painter();
    let c = pos2(r.min.x + 4.5, r.center().y);
    p.circle_stroke(c, 3.0, Stroke::new(1.5, col));
    p.line_segment([pos2(c.x + 3.0, c.y), pos2(r.max.x - 1.0, c.y)], Stroke::new(1.5, col));
    for x in [r.max.x - 2.0, r.max.x - 5.0] {
        p.line_segment([pos2(x, c.y), pos2(x, c.y + 3.0)], Stroke::new(1.5, col));
    }
    resp.on_hover_text(t!("Key Input"));
    ui.menu_button(egui::RichText::new(current).font(regular(11.0)), |ui| key_menu(app, ui, tid, slot, ins, &busses));
    let ext = ui
        .add_enabled(ins.key.is_some(), egui::Button::new(egui::RichText::new(t!("External Key")).font(regular(10.0))).fill(lit(ins.key_on)).small());
    if ext.clicked() {
        let _ = app.run("mix.insert_key", json!({"track": tid.0, "slot": slot, "on": !ins.key_on}));
    }
    let listen =
        ui.add_enabled(ins.key_on, egui::Button::new(egui::RichText::new(t!("Key Listen")).font(regular(10.0))).fill(lit(ins.key_listen)).small());
    if listen.clicked() {
        let _ = app.run("mix.insert_key", json!({"track": tid.0, "slot": slot, "listen": !ins.key_listen}));
    }
}

pub fn key_menu(app: &mut SoundApp, ui: &mut Ui, tid: TrackId, slot: usize, ins: &Insert, busses: &[(u64, String)]) {
    if ui.selectable_label(ins.key.is_none(), t!("no key input")).clicked() {
        let _ = app.run("mix.insert_key", json!({"track": tid.0, "slot": slot, "bus": null}));
        ui.close();
    }
    ui.separator();
    for (id, name) in busses {
        if ui.selectable_label(ins.key.is_some_and(|k| k.0 == *id), name).clicked() {
            let _ = app.run("mix.insert_key", json!({"track": tid.0, "slot": slot, "bus": id}));
            ui.close();
        }
    }
}

type Section = (&'static str, Color32, usize, &'static [&'static str]);

fn sections_of(id: &str) -> Option<(&'static [Section], bool)> {
    const GREY: Color32 = Color32::from_rgb(150, 154, 160);
    const BLUE: Color32 = Color32::from_rgb(76, 170, 222);
    const ORANGE: Color32 = Color32::from_rgb(232, 168, 60);
    const GREEN: Color32 = Color32::from_rgb(110, 196, 92);
    const PURPLE: Color32 = Color32::from_rgb(170, 120, 230);
    static STRIP: [Section; 5] = [
        ("INPUT", GREY, 1, &["input_gain"]),
        ("GATE", GREEN, 2, &["gate_on", "gate_threshold", "gate_range", "gate_release"]),
        ("COMP", ORANGE, 3, &["comp_on", "comp_threshold", "comp_ratio", "comp_attack", "comp_release", "comp_makeup"]),
        ("EQ", BLUE, 4, &["eq_on", "hpf_on", "hpf_freq", "low_gain", "low_freq", "mid_gain", "mid_freq", "mid_q", "high_gain", "high_freq"]),
        ("OUTPUT", GREY, 1, &["output_gain"]),
    ];
    static REVERB: [Section; 3] = [
        ("TIME", BLUE, 3, &["pre_delay", "decay", "size"]),
        ("COLOUR", PURPLE, 3, &["damping", "low_cut", "diffusion"]),
        ("OUTPUT", GREY, 2, &["width", "mix"]),
    ];
    static DELAY: [Section; 3] =
        [("DELAY", BLUE, 2, &["time", "feedback"]), ("MODULATION", PURPLE, 2, &["mod_rate", "mod_depth"]), ("OUTPUT", GREY, 2, &["lpf", "mix"])];
    static CHORUS: [Section; 3] =
        [("MODULATION", PURPLE, 2, &["rate", "depth"]), ("DELAY", BLUE, 2, &["delay", "feedback"]), ("OUTPUT", GREY, 2, &["spread", "mix"])];
    static FLANGER: [Section; 3] =
        [("MODULATION", PURPLE, 2, &["rate", "depth"]), ("DELAY", BLUE, 2, &["delay", "feedback"]), ("OUTPUT", GREY, 1, &["mix"])];
    static PHASER: [Section; 3] =
        [("MODULATION", PURPLE, 2, &["rate", "depth"]), ("FILTER", BLUE, 3, &["stages", "center", "feedback"]), ("OUTPUT", GREY, 1, &["mix"])];
    static SAT: [Section; 3] = [("DRIVE", ORANGE, 2, &["drive", "bias"]), ("TONE", BLUE, 1, &["tone"]), ("OUTPUT", GREY, 2, &["mix", "output"])];
    static DEESS: [Section; 2] = [("DETECTOR", ORANGE, 2, &["freq", "threshold"]), ("REDUCTION", BLUE, 2, &["range", "listen"])];
    static MAX: [Section; 2] = [("LIMITER", ORANGE, 2, &["threshold", "release"]), ("OUTPUT", GREY, 1, &["ceiling"])];
    Some(match id {
        "channel_strip" => (&STRIP[..], true),
        "room_reverb" | "plate_reverb" => (&REVERB[..], false),
        "mod_delay" => (&DELAY[..], false),
        "chorus" => (&CHORUS[..], false),
        "flanger" => (&FLANGER[..], false),
        "phaser" => (&PHASER[..], false),
        "saturator" => (&SAT[..], false),
        "de_esser" => (&DEESS[..], true),
        "maximizer" => (&MAX[..], true),
        _ => return None,
    })
}

fn sections_panel(ui: &mut Ui, info: &PluginInfo, ins: &Insert, gr: f32, sections: &[Section], meter: bool, set: &mut dyn FnMut(&str, f32)) {
    let col_w = 74.0;
    let reverb = matches!(info.id, "room_reverb" | "plate_reverb");
    let widths: Vec<f32> = sections.iter().map(|(_, _, cols, _)| *cols as f32 * col_w + 8.0).collect();
    let rows = sections.iter().map(|(_, _, cols, ids)| ids.len().div_ceil((*cols).max(1))).max().unwrap_or(1);
    let meter_w = if meter { 44.0 } else { 0.0 };
    let width = (widths.iter().sum::<f32>() + 24.0 + meter_w).max(360.0);
    let graph_h = if reverb { 120.0 } else { 0.0 };
    let height = 34.0 + graph_h + 22.0 + rows as f32 * CELL.y + 12.0;
    let r = frame(ui, info, vec2(width, height));
    let mut y = r.min.y + 34.0;
    if reverb {
        let g = Rect::from_min_size(pos2(r.min.x + 12.0, y), vec2(r.width() - 24.0, graph_h - 10.0));
        draw_decay(ui.painter(), g, info, ins);
        y += graph_h;
    }
    let mut x = r.min.x + 12.0;
    for ((title, col, cols, ids), w) in sections.iter().zip(&widths) {
        let head = Rect::from_min_size(pos2(x + 2.0, y), vec2(w - 4.0, 18.0));
        let p = ui.painter();
        p.rect_filled(head, 2.0, col.gamma_multiply(0.35));
        p.text(head.center(), Align2::CENTER_CENTER, *title, bold(10.5), *col);
        let body = Rect::from_min_max(pos2(x + 2.0, y + 20.0), pos2(x + w - 2.0, y + 22.0 + rows as f32 * CELL.y));
        p.rect_stroke(body, 2.0, Stroke::new(1.0, col.gamma_multiply(0.25)), StrokeKind::Inside);
        for (j, pid) in ids.iter().enumerate() {
            let Some(prm) = info.param(pid) else { continue };
            let c = j % (*cols).max(1);
            let row = j / (*cols).max(1);
            let cell = Rect::from_min_size(pos2(x + 4.0 + c as f32 * col_w, y + 22.0 + row as f32 * CELL.y), vec2(col_w, CELL.y));
            param_cell(ui, cell, prm, ins, set);
        }
        x += w;
    }
    if meter {
        let m = Rect::from_min_size(pos2(r.max.x - 12.0 - 14.0, y + 20.0), vec2(12.0, rows as f32 * CELL.y));
        gr_meter(ui.painter(), m, gr);
    }
}

fn gr_meter(p: &egui::Painter, m: Rect, gr: f32) {
    p.text(pos2(m.center().x, m.min.y - 8.0), Align2::CENTER_CENTER, t!("GR"), bold(9.0), LABEL);
    p.rect_filled(m, 2.0, READOUT_BG);
    let k = (gr.max(0.0) / 20.0).clamp(0.0, 1.0);
    if k > 0.0 {
        p.rect_filled(Rect::from_min_max(m.min, pos2(m.max.x, m.min.y + m.height() * k)), 2.0, Color32::from_rgb(232, 160, 40));
    }
    for db in [3.0f32, 6.0, 10.0, 15.0] {
        let y = m.min.y + m.height() * db / 20.0;
        p.hline(m.min.x - 4.0..=m.min.x - 1.0, y, Stroke::new(1.0, LABEL));
        p.text(pos2(m.min.x - 6.0, y), Align2::RIGHT_CENTER, format!("{db}"), regular(8.0), LABEL);
    }
}

fn draw_decay(p: &egui::Painter, g: Rect, info: &PluginInfo, ins: &Insert) {
    let val = |id: &str| ins.params.get(id).copied().or_else(|| info.param(id).map(|q| q.default)).unwrap_or(0.0);
    let (pre, decay, damp) = (val("pre_delay") / 1000.0, val("decay").max(0.1), val("damping"));
    let span = pre + decay * 1.25;
    p.rect(g, CornerRadius::same(2), Color32::from_rgb(12, 16, 20), Stroke::new(1.0, PANEL_EDGE), StrokeKind::Inside);
    let grid = Color32::from_rgb(38, 48, 58);
    for db in [-20.0f32, -40.0] {
        let y = g.min.y + g.height() * (-db / 60.0);
        p.hline(g.x_range(), y, Stroke::new(1.0, grid));
        p.text(pos2(g.min.x + 3.0, y - 6.0), Align2::LEFT_CENTER, format!("{db}"), regular(8.5), LABEL);
    }
    let curve = |rt: f32, col: Color32, w: f32| {
        let n = 160;
        let pts: Vec<egui::Pos2> = (0..n)
            .map(|i| {
                let t = span * i as f32 / (n - 1) as f32;
                let db = if t < pre { -60.0 } else { (-60.0 * (t - pre) / rt).max(-60.0) };
                pos2(g.min.x + g.width() * t / span, g.min.y + g.height() * (-db / 60.0))
            })
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(w, col)));
    };
    curve(decay * (damp / 20_000.0).clamp(0.1, 1.0), Color32::from_rgb(70, 100, 140), 1.5);
    curve(decay, Color32::from_rgb(120, 190, 255), 2.0);
    p.text(pos2(g.max.x - 6.0, g.min.y + 9.0), Align2::RIGHT_CENTER, format!("RT60 {decay:.2} s"), regular(9.0), LABEL);
}

pub fn panel_width(info: &PluginInfo) -> f32 {
    if crate::open_ui::handles(info.id) {
        return crate::open_ui::width(info.id);
    }
    match info.id {
        "eq_7band" => 9.0 * 78.0 + 24.0,
        "compressor" | "expander_gate" => (12.0 + 200.0 + 50.0 + 3.0 * CELL.x + 12.0).max(5.0 * CELL.x + 24.0),
        id => match sections_of(id) {
            Some((sections, meter)) => {
                let w: f32 = sections.iter().map(|(_, _, cols, _)| *cols as f32 * 74.0 + 8.0).sum();
                (w + 24.0 + if meter { 44.0 } else { 0.0 }).max(360.0)
            }
            None => (info.params.len().clamp(1, 6) as f32 * CELL.x + 24.0).max(360.0),
        },
    }
}

fn automation_dialog_id() -> egui::Id {
    egui::Id::new("plugin-automation-dialog")
}

pub fn open_automation_dialog(ctx: &egui::Context, tid: TrackId, slot: usize) {
    ctx.data_mut(|d| d.insert_temp(automation_dialog_id(), (tid.0, slot)));
}

pub fn automation_dialog(app: &mut SoundApp, ctx: &egui::Context) {
    let Some((t, slot)) = ctx.data(|d| d.get_temp::<(u64, usize)>(automation_dialog_id())) else { return };
    let tid = TrackId(t);
    let Some((tname, ins)) =
        app.engine.session().track(tid).and_then(|tr| tr.mixer.inserts.get(slot).cloned().flatten().map(|i| (tr.name.clone(), i)))
    else {
        ctx.data_mut(|d| d.remove::<(u64, usize)>(automation_dialog_id()));
        return;
    };
    let Some(info) = crate::mix_window::plugin_info(&ins.plugin) else { return };
    let all: Vec<(&str, &str)> =
        std::iter::once((openpt_model::BYPASS_PARAM, "Master Bypass")).chain(info.params.iter().map(|p| (p.id, p.name))).collect();
    let (left, right): (Vec<_>, Vec<_>) = all.iter().partition(|(id, _)| !ins.is_automated(id));
    let sel_id = egui::Id::new(("plugin-automation-sel", t, slot));
    let mut sel: (Vec<String>, Vec<String>) = ctx.data(|d| d.get_temp(sel_id)).unwrap_or_default();
    let mut open = true;
    let mut done = false;
    let mut change: Option<(Vec<String>, bool)> = None;
    egui::Window::new(format!("Plug-in Automation · {tname} · {}", info.name))
        .id(automation_dialog_id())
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .fit_screen(ctx)
        .show(ctx, |ui| {
            let list = |ui: &mut Ui, items: &[&(&str, &str)], chosen: &mut Vec<String>, salt: &str| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_size(vec2(200.0, 260.0));
                    egui::ScrollArea::vertical().id_salt(salt).max_height(260.0).show(ui, |ui| {
                        for (id, name) in items {
                            let on = chosen.iter().any(|c| c == id);
                            let r = ui.selectable_label(on, *name);
                            if r.clicked() {
                                if ui.input(|i| i.modifiers.command || i.modifiers.shift) {
                                    if on {
                                        chosen.retain(|c| c != id);
                                    } else {
                                        chosen.push((*id).to_string());
                                    }
                                } else {
                                    *chosen = vec![(*id).to_string()];
                                }
                            }
                            if r.double_clicked() {
                                *chosen = vec![(*id).to_string()];
                            }
                        }
                    });
                });
            };
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(t!("Controls"));
                    list(ui, &left, &mut sel.0, "auto-left");
                });
                ui.vertical(|ui| {
                    ui.add_space(110.0);
                    if ui.add_enabled(!sel.0.is_empty(), egui::Button::new(t!("Add >>"))).clicked() {
                        change = Some((std::mem::take(&mut sel.0), true));
                    }
                    if ui.add_enabled(!sel.1.is_empty(), egui::Button::new(t!("<< Remove"))).clicked() {
                        change = Some((std::mem::take(&mut sel.1), false));
                    }
                });
                ui.vertical(|ui| {
                    ui.label(t!("Automation enabled"));
                    list(ui, &right, &mut sel.1, "auto-right");
                });
            });
            ui.horizontal(|ui| {
                if ui.button(t!("Add All")).clicked() {
                    change = Some((left.iter().map(|(id, _)| (*id).to_string()).collect(), true));
                }
                if ui.button(t!("Remove All")).clicked() {
                    change = Some((right.iter().map(|(id, _)| (*id).to_string()).collect(), false));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| done = ui.button(t!("Done")).clicked());
            });
        });
    if let Some((ids, enable)) = change
        && !ids.is_empty()
        && let Err(e) = app.run("mix.insert_automation", json!({"track": t, "slot": slot, "params": ids, "enable": enable}))
    {
        app.ui.status = e;
    }
    if !open || done {
        ctx.data_mut(|d| {
            d.remove::<(u64, usize)>(automation_dialog_id());
            d.remove::<(Vec<String>, Vec<String>)>(sel_id);
        });
    } else {
        ctx.data_mut(|d| d.insert_temp(sel_id, sel));
    }
}
