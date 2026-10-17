pub mod comp;
pub mod drt;
pub mod eq;
pub mod mb;
pub mod sat;

use crate::theme::{bold, regular};
use egui::{Align2, Color32, CornerRadius, Id, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use openpt_dsp::scope::Scope;
use openpt_dsp::{ParamInfo, PluginInfo, Unit};
use std::any::Any;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

pub const PLATE: Color32 = Color32::from_rgb(27, 27, 31);
pub const PLATE_HI: Color32 = Color32::from_rgb(36, 36, 41);
pub const HEADER: Color32 = Color32::from_rgb(21, 20, 25);
pub const EDGE: Color32 = Color32::from_rgb(10, 10, 12);
pub const DISPLAY: Color32 = Color32::from_rgb(14, 13, 19);
pub const GRID: Color32 = Color32::from_rgb(38, 36, 52);
pub const GRID_STRONG: Color32 = Color32::from_rgb(58, 54, 80);
pub const TEXT: Color32 = Color32::from_rgb(218, 218, 222);
pub const DIM: Color32 = Color32::from_rgb(138, 136, 150);
pub const ACCENT: Color32 = Color32::from_rgb(184, 164, 255);
pub const ACCENT_DEEP: Color32 = Color32::from_rgb(74, 47, 143);
pub const READOUT: Color32 = Color32::from_rgb(56, 209, 119);
pub const READOUT_BG: Color32 = Color32::from_rgb(6, 8, 7);
pub const GR: Color32 = Color32::from_rgb(240, 120, 70);
pub const OFF: Color32 = Color32::from_rgb(48, 47, 54);

pub const BAND_COLORS: [Color32; 8] = [
    Color32::from_rgb(184, 164, 255),
    Color32::from_rgb(90, 200, 250),
    Color32::from_rgb(86, 214, 140),
    Color32::from_rgb(236, 206, 84),
    Color32::from_rgb(244, 140, 82),
    Color32::from_rgb(240, 96, 128),
    Color32::from_rgb(200, 120, 230),
    Color32::from_rgb(120, 150, 255),
];

pub fn band_color(b: usize) -> Color32 {
    BAND_COLORS[b % BAND_COLORS.len()]
}

pub fn handles(id: &str) -> bool {
    matches!(id, "open_eq" | "open_mb" | "open_comp" | "open_sat" | "open_drt")
}

pub fn width(id: &str) -> f32 {
    match id {
        "open_eq" => eq::W,
        "open_mb" => mb::W,
        "open_comp" => comp::W,
        "open_sat" => sat::W,
        "open_drt" => drt::W,
        _ => 400.0,
    }
}

pub struct Ctx<'a> {
    pub info: &'static PluginInfo,
    pub params: &'a BTreeMap<String, f32>,
    pub set: &'a mut dyn FnMut(&str, f32),
    pub scope: Option<Arc<Scope>>,
    pub id: Id,
}

impl Ctx<'_> {
    pub fn get(&self, id: &str) -> f32 {
        self.params.get(id).copied().unwrap_or_else(|| self.info.param(id).map_or(0.0, |p| p.default))
    }
    pub fn on(&self, id: &str) -> bool {
        self.get(id) >= 0.5
    }
    pub fn idx(&self, id: &str) -> usize {
        self.get(id).round().max(0.0) as usize
    }
    pub fn set(&mut self, id: &str, v: f32) {
        (self.set)(id, v);
    }
    pub fn rate(&self) -> f32 {
        self.scope.as_ref().map_or(48_000.0, |s| s.read(|d| d.rate)).max(8_000.0)
    }
    pub fn param(&self, id: &str) -> Option<&'static ParamInfo> {
        self.info.param(id)
    }
}

pub fn panel(ui: &mut Ui, info: &'static PluginInfo, params: &BTreeMap<String, f32>, scope: Option<Arc<Scope>>, salt: Id, set: &mut dyn FnMut(&str, f32)) -> bool {
    if !handles(info.id) {
        return false;
    }
    let mut cx = Ctx { info, params, set, scope, id: salt.with(info.id) };
    match info.id {
        "open_eq" => eq::show(ui, &mut cx),
        "open_mb" => mb::show(ui, &mut cx),
        "open_comp" => comp::show(ui, &mut cx),
        "open_sat" => sat::show(ui, &mut cx),
        "open_drt" => drt::show(ui, &mut cx),
        _ => {}
    }
    if cx.scope.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
    true
}

thread_local! {
    static STATES: RefCell<HashMap<Id, Box<dyn Any>>> = RefCell::new(HashMap::new());
}

pub fn with_state<T: Default + 'static, R>(id: Id, f: impl FnOnce(&mut T) -> R) -> R {
    STATES.with(|s| {
        let mut map = s.borrow_mut();
        let entry = map.entry(id).or_insert_with(|| Box::new(T::default()));
        if !entry.is::<T>() {
            *entry = Box::new(T::default());
        }
        match entry.downcast_mut::<T>() {
            Some(t) => f(t),
            None => f(&mut T::default()),
        }
    })
}

pub fn plate(ui: &mut Ui, size: egui::Vec2, name: &str) -> (Rect, Rect) {
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = &ui.painter().clone();
    p.rect(r, CornerRadius::same(4), PLATE, Stroke::new(1.0, EDGE), StrokeKind::Inside);
    let head = Rect::from_min_size(r.min, vec2(r.width(), 30.0));
    p.rect_filled(head.shrink(1.0), CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, HEADER);
    p.hline(head.x_range(), head.max.y, Stroke::new(1.0, EDGE));
    let m = Rect::from_center_size(pos2(head.min.x + 18.0, head.center().y), vec2(16.0, 16.0));
    p.rect_filled(m, 3.0, ACCENT_DEEP);
    let pts: Vec<Pos2> = (0..=12)
        .map(|i| {
            let t = i as f32 / 12.0;
            pos2(m.min.x + 2.0 + t * 12.0, m.center().y - (t * 9.0).sin() * 4.0 * (1.0 - (2.0 * t - 1.0).abs()))
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.4, ACCENT)));
    let open = p.text(pos2(m.max.x + 8.0, head.center().y), Align2::LEFT_CENTER, "Open", regular(15.0), TEXT);
    p.text(pos2(open.max.x, head.center().y), Align2::LEFT_CENTER, name, bold(15.0), ACCENT);
    p.text(pos2(head.max.x - 12.0, head.center().y), Align2::RIGHT_CENTER, "OpenPT", regular(10.0), DIM);
    (Rect::from_min_max(pos2(r.min.x, head.max.y + 1.0), r.max), head)
}

pub fn display(p: &Painter, r: Rect) {
    p.rect(r, 3.0, DISPLAY, Stroke::new(1.0, EDGE), StrokeKind::Outside);
}

pub fn readout(p: &Painter, r: Rect, text: &str) {
    p.rect_filled(r, 2.0, READOUT_BG);
    p.text(r.center(), Align2::CENTER_CENTER, text, regular(10.5), READOUT);
}

pub fn caption(p: &Painter, at: Pos2, text: &str) {
    p.text(at, Align2::CENTER_CENTER, text.to_uppercase(), regular(9.0), DIM);
}

pub fn button(ui: &mut Ui, r: Rect, text: &str, on: bool, lit: Color32) -> Response {
    let resp = ui.interact(r, ui.id().with(("open-btn", r.min.x as i32, r.min.y as i32)), Sense::click());
    let p = &ui.painter().clone();
    let fill = if on { lit } else if resp.hovered() { PLATE_HI } else { OFF };
    p.rect(r, 3.0, fill, Stroke::new(1.0, EDGE), StrokeKind::Inside);
    let tc = if on { Color32::from_rgb(16, 14, 22) } else { TEXT };
    p.text(r.center(), Align2::CENTER_CENTER, text, bold(10.0), tc);
    resp
}

pub fn choice(ui: &mut Ui, r: Rect, salt: impl std::hash::Hash + std::fmt::Debug, choices: &[&str], idx: usize) -> Option<usize> {
    let mut picked = None;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().button_padding = vec2(4.0, 1.0);
    let label = choices.get(idx).copied().unwrap_or("?");
    egui::ComboBox::from_id_salt(Id::new(("open-choice", salt)))
        .width(r.width() - 6.0)
        .selected_text(egui::RichText::new(label).font(regular(11.0)).color(TEXT))
        .show_ui(&mut child, |ui| {
            for (k, c) in choices.iter().enumerate() {
                if ui.selectable_label(k == idx, *c).clicked() {
                    picked = Some(k);
                }
            }
        });
    picked
}

pub fn knob(ui: &mut Ui, cx: &mut Ctx, cell: Rect, id: &str, label: &str, color: Color32) {
    let Some(prm) = cx.param(id) else { return };
    let v = cx.get(id);
    knob_with(ui, cx, cell, prm, v, label, color, &|v| prm.format(v));
}

pub fn knob_with(ui: &mut Ui, cx: &mut Ctx, cell: Rect, prm: &ParamInfo, v: f32, label: &str, color: Color32, text: &dyn Fn(f32) -> String) {
    let size = (cell.width() - 18.0).min(cell.height() - 34.0).clamp(20.0, 52.0);
    let c = pos2(cell.center().x, cell.min.y + 14.0 + size * 0.5);
    let kr = Rect::from_center_size(c, vec2(size, size));
    let resp = ui.interact(kr, cx.id.with(("knob", prm.id)), Sense::click_and_drag());
    let mut n = prm.to_normalized(v);
    let mut changed = false;
    if resp.dragged() {
        let fine = if ui.input(|i| i.modifiers.shift || i.modifiers.command) { 0.15 } else { 1.0 };
        n = (n - resp.drag_delta().y * 0.005 * fine).clamp(0.0, 1.0);
        changed = true;
    }
    if resp.hovered() {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        if wheel != 0.0 {
            n = (n + wheel * 0.0015).clamp(0.0, 1.0);
            changed = true;
        }
    }
    if resp.double_clicked() || (resp.clicked() && ui.input(|i| i.modifiers.alt)) {
        n = prm.to_normalized(prm.default);
        changed = true;
    }
    if changed {
        let nv = prm.from_normalized(n);
        if (nv - v).abs() > f32::EPSILON {
            cx.set(prm.id, nv);
        }
    }
    let p = &ui.painter().clone();
    caption(p, pos2(cell.center().x, cell.min.y + 6.0), label);
    let rad = size * 0.5 - 2.0;
    let start = std::f32::consts::PI * 0.75;
    let sweep = std::f32::consts::PI * 1.5;
    let arc = |from: f32, to: f32| -> Vec<Pos2> {
        let steps = 40;
        (0..=steps)
            .map(|i| {
                let a = start + sweep * (from + (to - from) * i as f32 / steps as f32);
                c + vec2(a.cos(), a.sin()) * rad
            })
            .collect()
    };
    p.add(egui::Shape::line(arc(0.0, 1.0), Stroke::new(3.0, OFF)));
    let bipolar = prm.min < 0.0 && prm.max > 0.0 && prm.unit == Unit::Db;
    let from = if bipolar { prm.to_normalized(0.0) } else { 0.0 };
    let (a, b) = if n < from { (n, from) } else { (from, n) };
    if b - a > 0.001 {
        p.add(egui::Shape::line(arc(a, b), Stroke::new(3.0, color)));
    }
    let face = if resp.hovered() || resp.dragged() { PLATE_HI } else { Color32::from_rgb(32, 32, 37) };
    p.circle(c, rad - 4.0, face, Stroke::new(1.0, EDGE));
    let ang = start + sweep * n;
    let d = vec2(ang.cos(), ang.sin());
    p.line_segment([c + d * (rad - 13.0).max(2.0), c + d * (rad - 5.0)], Stroke::new(2.0, TEXT));
    readout(p, Rect::from_center_size(pos2(cell.center().x, kr.max.y + 10.0), vec2((cell.width() - 6.0).min(78.0), 15.0)), &text(v));
    resp.on_hover_text(prm.name);
}

#[derive(Clone, Copy)]
pub struct FreqAxis {
    pub r: Rect,
    pub lo: f32,
    pub hi: f32,
}

impl FreqAxis {
    pub fn x(&self, f: f32) -> f32 {
        self.r.min.x + self.r.width() * ((f.max(1.0) / self.lo).ln() / (self.hi / self.lo).ln())
    }
    pub fn f(&self, x: f32) -> f32 {
        self.lo * (self.hi / self.lo).powf(((x - self.r.min.x) / self.r.width()).clamp(-0.1, 1.1))
    }
    pub fn grid(&self, p: &Painter) {
        for dec in [10.0f32, 100.0, 1000.0, 10_000.0] {
            for k in 1..10 {
                let f = dec * k as f32;
                if f < self.lo || f > self.hi {
                    continue;
                }
                let x = self.x(f);
                p.vline(x, self.r.y_range(), Stroke::new(1.0, if k == 1 { GRID_STRONG } else { GRID }));
            }
        }
        for (f, t) in [(20.0, "20"), (50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10_000.0, "10k"), (20_000.0, "20k")] {
            if f >= self.lo && f <= self.hi {
                p.text(pos2(self.x(f) + 3.0, self.r.max.y - 3.0), Align2::LEFT_BOTTOM, t, regular(9.0), DIM);
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct DbAxis {
    pub r: Rect,
    pub lo: f32,
    pub hi: f32,
}

impl DbAxis {
    pub fn y(&self, db: f32) -> f32 {
        self.r.max.y - self.r.height() * (db - self.lo) / (self.hi - self.lo)
    }
    pub fn db(&self, y: f32) -> f32 {
        self.lo + (self.r.max.y - y) / self.r.height() * (self.hi - self.lo)
    }
    pub fn grid(&self, p: &Painter, step: f32, right_labels: bool) {
        let mut d = (self.lo / step).ceil() * step;
        while d <= self.hi + 0.01 {
            let y = self.y(d);
            p.hline(self.r.x_range(), y, Stroke::new(1.0, if d.abs() < 0.01 { GRID_STRONG } else { GRID }));
            let t = if d > 0.0 { format!("+{d:.0}") } else { format!("{d:.0}") };
            let (x, al) = if right_labels { (self.r.max.x - 3.0, Align2::RIGHT_CENTER) } else { (self.r.min.x + 3.0, Align2::LEFT_CENTER) };
            p.text(pos2(x, y), al, t, regular(9.0), DIM);
            d += step;
        }
    }
}

pub fn spectrum(p: &Painter, fa: &FreqAxis, da: &DbAxis, cols: &[f32], fill: Color32, line: Color32) {
    if cols.len() < 2 {
        return;
    }
    let step = fa.r.width() / (cols.len() - 1) as f32;
    let pts: Vec<Pos2> = cols.iter().enumerate().map(|(i, &d)| pos2(fa.r.min.x + i as f32 * step, da.y(d).clamp(fa.r.min.y, fa.r.max.y))).collect();
    if fill.a() > 0 {
        let mut mesh = egui::Mesh::default();
        for (i, q) in pts.iter().enumerate() {
            mesh.colored_vertex(*q, fill);
            mesh.colored_vertex(pos2(q.x, fa.r.max.y), fill.gamma_multiply(0.2));
            if i > 0 {
                let k = (2 * i) as u32;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k, k + 1);
            }
        }
        p.add(mesh);
    }
    p.add(egui::Shape::line(pts, Stroke::new(1.0, line)));
}

#[derive(Default)]
pub struct Smoothed {
    pub pre: Vec<f32>,
    pub post: Vec<f32>,
    pub analyzer: Option<openpt_dsp::scope::Analyzer>,
}

impl Smoothed {
    pub fn update(&mut self, scope: &Scope, fa: &FreqAxis, cols: usize, pre: bool, post: bool) {
        let an = self.analyzer.get_or_insert_with(|| openpt_dsp::scope::Analyzer::new(8192));
        for (on, which) in [(pre, false), (post, true)] {
            let v = if which { &mut self.post } else { &mut self.pre };
            if !on {
                v.clear();
                continue;
            }
            if v.len() != cols {
                *v = vec![-120.0; cols];
            }
            let rate = scope.read(|d| {
                an.run(d, which);
                d.rate
            });
            let w = fa.r.width();
            for (i, out) in v.iter_mut().enumerate() {
                let x = fa.r.min.x + w * i as f32 / (cols - 1).max(1) as f32;
                let dx = w / cols as f32;
                let f = fa.f(x);
                let d = an.db_at(f, fa.f(x - dx), fa.f(x + dx), rate) + 4.5 * (f / 1000.0).log2();
                *out += (if d > *out { 0.6 } else { 0.12 }) * (d - *out);
            }
        }
    }
}
