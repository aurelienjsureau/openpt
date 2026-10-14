use openpt_ui_kit::buttons::{ButtonVisuals, IconButton, IconButtonStyle};

use crate::icons;
use crate::theme::{Tokens, bold, regular};
use egui::{Align2, Color32, CornerRadius, Mesh, Painter, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

pub fn shade(c: Color32, k: f32) -> Color32 {
    if k >= 0.0 { c.lerp_to_gamma(Color32::WHITE, k) } else { c.lerp_to_gamma(Color32::BLACK, -k) }
}

fn quad(mesh: &mut Mesh, r: Rect, [tl, tr, bl, br]: [Color32; 4]) {
    let i = mesh.vertices.len() as u32;
    for (p, c) in [(r.left_top(), tl), (r.right_top(), tr), (r.left_bottom(), bl), (r.right_bottom(), br)] {
        mesh.colored_vertex(p, c);
    }
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i + 1, i + 2, i + 3);
}

pub fn vgradient(p: &Painter, r: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    quad(&mut mesh, r, [top, top, bottom, bottom]);
    p.add(mesh);
}

pub fn rounded_vgradient(p: &Painter, r: Rect, radius: f32, top: Color32, bottom: Color32) {
    let options = p.ctx().tessellation_options(|o| *o);
    let mut tessellator = egui::epaint::Tessellator::new(p.pixels_per_point(), options, [1, 1], Vec::new());
    let mut mesh = Mesh::default();
    tessellator.tessellate_rect(&egui::epaint::RectShape::filled(r, radius, Color32::WHITE), &mut mesh);
    for v in &mut mesh.vertices {
        let k = ((v.pos.y - r.min.y) / r.height()).clamp(0.0, 1.0);
        v.color = top.lerp_to_gamma(bottom, k).gamma_multiply(v.color.a() as f32 / 255.0);
    }
    p.add(mesh);
}

pub fn raised_key(p: &Painter, r: Rect, faces: [Color32; 2], hovered: bool) -> Color32 {
    let [top, bottom] = if hovered { faces.map(|c| shade(c, 0.06)) } else { faces };
    p.rect_filled(r, 3.0, shade(Tokens::current().slot_well, -0.3));
    rounded_vgradient(p, r.shrink(1.0), 2.0, top, bottom);
    p.hline(r.min.x + 2.0..=r.max.x - 2.0, r.min.y + 1.5, Stroke::new(1.0, shade(top, 0.17)));
    top
}

pub fn groove(p: &Painter, x: f32, ys: impl Into<egui::Rangef>, top: Color32) {
    let ys = ys.into();
    p.vline(x, ys, Stroke::new(1.0, shade(top, -0.42)));
    p.vline(x + 1.0, ys, Stroke::new(1.0, shade(top, 0.12)));
}

pub fn inset(p: &Painter, r: Rect, fill: Color32) {
    let t = Tokens::current();
    p.rect(r, 2.0, fill, Stroke::new(1.0, t.border), StrokeKind::Inside);
    p.hline(r.min.x + 1.0..=r.max.x - 1.0, r.min.y + 1.5, Stroke::new(1.0, Color32::from_black_alpha(60)));
    p.hline(r.min.x + 2.0..=r.max.x - 2.0, r.max.y + 0.5, Stroke::new(1.0, t.edge_light));
}

pub fn key_fill(resp: &Response) -> Color32 {
    let t = Tokens::current();
    if resp.hovered() { t.button_hi } else { t.button }
}

pub fn key(p: &Painter, r: Rect, resp: &Response, fill: Color32) {
    let face = r.shrink(1.0);
    if resp.is_pointer_button_down_on() {
        vgradient(p, face, shade(fill, -0.14), shade(fill, 0.02));
    } else {
        p.rect_filled(face, 0.0, fill);
        key_light(p, face);
    }
    p.rect_stroke(r, 3.0, Stroke::new(1.0, Tokens::current().button_border), StrokeKind::Inside);
}

fn key_light(p: &Painter, face: Rect) {
    vgradient(p, face, Color32::from_white_alpha(26), Color32::from_black_alpha(31));
    p.hline(face.min.x + 1.0..=face.max.x - 1.0, face.min.y + 0.5, Stroke::new(1.0, Color32::from_white_alpha(30)));
}

fn button_style(selected_fill: Color32, normal_icon: Color32, radius: u8) -> IconButtonStyle {
    let t = Tokens::current();
    let visuals = |fill, icon| ButtonVisuals { fill, stroke: Stroke::new(1.0, t.button_border), icon };
    IconButtonStyle {
        normal: visuals(t.button, normal_icon),
        hovered: visuals(t.button_hi, normal_icon),
        pressed: Some(visuals(shade(t.button, -0.1), normal_icon)),
        selected: visuals(selected_fill, Color32::WHITE),
        corner_radius: CornerRadius::same(radius),
        focus_stroke: Stroke::new(1.0, t.accent),
    }
}

pub fn icon_button(ui: &mut Ui, size: egui::Vec2, icon: &str, on: bool, tip: &str) -> Response {
    let t = Tokens::current();
    let style = button_style(t.accent, t.text, 3);
    let resp = IconButton::new(tip, size, &style).selected(on).single_shape_frame().show(ui, |ui, r, tint| {
        key_light(ui.painter(), r.shrink(1.0));
        let s = r.height().min(r.width()) * 0.78;
        icons::draw(ui.painter(), Rect::from_center_size(r.center(), vec2(s, s)), icon, tint);
    });
    resp.on_hover_text(tip)
}

pub fn text_toggle(ui: &mut Ui, size: egui::Vec2, text: &str, on: bool, on_color: Color32, tip: &str) -> Response {
    let t = Tokens::current();
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    key(ui.painter(), r, &resp, if on { on_color } else { key_fill(&resp) });
    let col = if on { t.text_dark } else { t.text };
    ui.painter().text(r.center(), Align2::CENTER_CENTER, text, bold((size.y * 0.62).clamp(8.0, 13.0)), col);
    resp.on_hover_text(tip)
}

pub fn rec_toggle(ui: &mut Ui, size: egui::Vec2, on: bool, tip: &str) -> Response {
    let t = Tokens::current();
    let style = button_style(t.rec, Color32::from_rgb(218, 112, 106), 2);
    let resp = IconButton::new(tip, size, &style).selected(on).single_shape_frame().show(ui, |ui, r, tint| {
        key_light(ui.painter(), r.shrink(1.0));
        let rad = size.y * 0.24;
        if on {
            ui.painter().circle_filled(r.center(), rad, tint);
        } else {
            rounded_vgradient(
                ui.painter(),
                Rect::from_center_size(r.center(), Vec2::splat(2.0 * rad)),
                2.0 * rad,
                tint,
                Color32::from_rgb(176, 76, 72),
            );
        }
        ui.painter().circle_stroke(r.center(), rad, Stroke::new(1.0, Color32::from_black_alpha(90)));
    });
    resp.on_hover_text(tip)
}

pub fn selector_box(ui: &mut Ui, width: f32, height: f32, text: &str, color: Color32) -> Response {
    selector_box_with_tooltip(ui, width, height, text, color, text)
}

pub(crate) fn selector_box_with_tooltip(ui: &mut Ui, width: f32, height: f32, text: &str, color: Color32, tooltip: &str) -> Response {
    let t = Tokens::current();
    let (r, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    key(ui.painter(), r, &resp, key_fill(&resp));
    let tr = Rect::from_min_max(pos2(r.max.x - height * 0.8, r.min.y), r.max);
    icons::draw(ui.painter(), tr.shrink(height * 0.15), "triangle_down", t.text_dim);
    let text_rect = Rect::from_min_max(r.min + vec2(3.0, 1.0), pos2(tr.left() - 2.0, r.bottom() - 1.0));
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), regular((height * 0.62).clamp(8.0, 12.0)), color, text_rect.width().max(1.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.painter().layout_job(job);
    let pos = text_rect.center() - galley.size() * 0.5;
    ui.painter().with_clip_rect(r.shrink(1.0)).galley(pos, galley, color);
    resp.on_hover_text(tooltip)
}

pub fn pan_knob(ui: &mut Ui, size: f32, value: &mut f32, tip: &str) -> Response {
    let t = Tokens::current();
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    if resp.dragged() {
        let d = resp.drag_delta();
        *value = (*value + (d.x - d.y) * 0.01).clamp(-1.0, 1.0);
    }
    if resp.double_clicked() {
        *value = 0.0;
    }
    let p = ui.painter();
    let c = r.center();
    let rad = size * 0.42;
    let n = 24;
    let start = std::f32::consts::PI * 0.75;
    let sweep = std::f32::consts::PI * 1.5;
    let arc = |a0: f32, a1: f32| -> Vec<egui::Pos2> {
        (0..=n)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / n as f32;
                c + vec2(a.cos(), a.sin()) * (rad + 2.5)
            })
            .collect()
    };
    let mid = start + sweep * 0.5;
    let at = start + sweep * (*value * 0.5 + 0.5);
    let (a0, a1) = if at < mid { (at, mid) } else { (mid, at) };
    p.add(egui::Shape::line(arc(start, start + sweep), Stroke::new(2.0, t.meter_bg)));
    p.add(egui::Shape::line(arc(a0, a1), Stroke::new(2.0, t.counter_text)));
    p.circle_filled(c + vec2(0.0, 1.0), rad, Color32::from_black_alpha(90));
    rounded_vgradient(p, Rect::from_center_size(c, Vec2::splat(2.0 * rad)), 2.0 * rad, shade(t.knob_bg, 0.14), shade(t.knob_bg, -0.2));
    p.circle_stroke(c, rad - 0.75, Stroke::new(1.5, t.knob_ring));
    let d = vec2(at.cos(), at.sin());
    p.line_segment([c + d * rad * 0.15, c + d * (rad - 2.5)], Stroke::new(2.0, t.knob_pointer));
    resp.on_hover_text(tip)
}

pub fn pan_text(v: f32) -> String {
    let n = (v * 100.0).round() as i32;
    match n {
        0 => "0".into(),
        n if n < 0 => format!("<{}", -n),
        n => format!("{n}>"),
    }
}

const METER_MARKS: [f32; 8] = [0.0, -5.0, -10.0, -15.0, -20.0, -30.0, -40.0, -60.0];

fn meter_bar(r: Rect) -> Rect {
    Rect::from_min_max(pos2(r.min.x + 1.0, r.min.y + 5.0), pos2(r.max.x - 1.0, r.max.y - 1.0))
}

fn meter_marks(bar: Rect) -> impl Iterator<Item = (f32, f32)> {
    let mut last = f32::NEG_INFINITY;
    METER_MARKS.into_iter().filter_map(move |db| {
        let y = bar.max.y - bar.height() * meter_pos(db);
        (y - last >= 9.0).then(|| {
            last = y;
            (db, y)
        })
    })
}

pub fn meter(ui: &Ui, r: Rect, level: f32, hold: f32, clip: bool) {
    let t = Tokens::current();
    let p = ui.painter();
    p.rect_filled(Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + 3.0)), 1.0, if clip { t.meter_red } else { t.meter_clip_off });
    p.rect_filled(Rect::from_min_max(pos2(r.min.x, r.min.y + 4.0), r.max), 0.0, t.meter_bg);
    let bar = meter_bar(r);
    let y = |pos: f32| bar.max.y - bar.height() * pos;
    let (amber, red) = (meter_pos(-12.0), meter_pos(-3.0));
    let zones = [(0.0, amber, t.meter_green), (amber, red, t.meter_yellow), (red, 1.0, t.meter_red)];
    let lit = level_pos(level);
    let unlit = if t.meter_bg.intensity() > 0.5 { 0.08 } else { 0.18 };
    let mut mesh = Mesh::default();
    for (lo, hi, color) in zones {
        let top = lit.clamp(lo, hi);
        quad(&mut mesh, Rect::from_x_y_ranges(bar.x_range(), y(top)..=y(lo)), [color; 4]);
        quad(&mut mesh, Rect::from_x_y_ranges(bar.x_range(), y(hi)..=y(top)), [color.gamma_multiply(unlit); 4]);
    }
    p.add(mesh);
    for (_, at) in meter_marks(bar).skip(1) {
        p.hline(bar.x_range(), p.round_to_pixel_center(at), Stroke::new(1.0, Color32::from_black_alpha(90)));
    }
    let held = level_pos(hold);
    if held > 0.01 {
        let color = zones.iter().find(|z| held <= z.1).map_or(t.meter_red, |z| z.2);
        p.rect_filled(Rect::from_x_y_ranges(bar.x_range(), y(held) - 1.0..=y(held) + 1.0), 0.0, color);
    }
}

pub fn meter_scale(p: &Painter, r: Rect, x: f32) {
    let t = Tokens::current();
    for (db, y) in meter_marks(meter_bar(r)) {
        p.text(pos2(x - 3.0, y), Align2::RIGHT_CENTER, format!("{}", db.abs()), regular(8.0), t.text_dim);
        p.hline(x - 2.0..=x, y, Stroke::new(1.0, t.text_dim.gamma_multiply(0.7)));
    }
}

pub fn level_pos(v: f32) -> f32 {
    meter_pos(if v <= 1e-6 { -80.0 } else { 20.0 * v.log10() })
}

pub fn meter_pos(db: f32) -> f32 {
    if !db.is_finite() || db <= -80.0 {
        return 0.0;
    }
    if db >= 0.0 {
        return 1.0;
    }
    if db < -40.0 {
        (db + 80.0) / 40.0 * 0.1
    } else if db < -20.0 {
        0.1 + (db + 40.0) / 20.0 * 0.2
    } else {
        0.3 + (db + 20.0) / 20.0 * 0.7
    }
}

pub fn clip_gain_from_pos(pos: f32) -> f32 {
    let p = if pos.is_finite() { pos.clamp(0.0, 1.0) } else { 0.0 };
    if p >= 0.25 { -36.0 + (p - 0.25) / 0.75 * 72.0 } else { -36.0 - (1.0 - p / 0.25).powf(1.5) * 108.0 }
}

pub fn clip_gain_to_pos(db: f32) -> f32 {
    let db = if db.is_finite() { db.clamp(-144.0, 36.0) } else { -144.0 };
    if db >= -36.0 { 0.25 + (db + 36.0) / 72.0 * 0.75 } else { (1.0 - ((-36.0 - db) / 108.0).powf(1.0 / 1.5)) * 0.25 }
}

const FADER_CAP_H: f32 = 28.0;

pub fn fader_slot(r: Rect) -> Rect {
    Rect::from_center_size(r.center(), vec2(4.0, r.height() - FADER_CAP_H))
}

pub fn fader(ui: &mut Ui, r: Rect, db: f32, id: egui::Id) -> Option<f32> {
    let t = Tokens::current();
    let resp = ui.interact(r, id, Sense::click_and_drag());
    let p = ui.painter();
    let slot = fader_slot(r);
    let cap_w = (r.width() * 0.4).clamp(13.0, 15.0);
    p.rect_filled(slot, 2.0, t.fader_track);
    p.vline(slot.min.x + 0.5, slot.min.y + 1.0..=slot.max.y - 1.0, Stroke::new(1.0, Color32::from_black_alpha(120)));
    p.vline(slot.max.x + 0.5, slot.min.y + 2.0..=slot.max.y - 2.0, Stroke::new(1.0, Color32::from_white_alpha(18)));
    let numbers_x = slot.center().x - cap_w * 0.5 - 1.0;
    let mut last = f32::NEG_INFINITY;
    for mark in [12.0f32, 6.0, 0.0, -5.0, -10.0, -15.0, -20.0, -30.0, -40.0, -60.0] {
        let y = slot.max.y - slot.height() * openpt_model::fader_db_to_pos(mark);
        p.hline(slot.min.x - 6.0..=slot.min.x - 2.0, y, Stroke::new(1.0, t.text_dim.gamma_multiply(0.7)));
        if numbers_x - r.min.x >= 10.0 && y - last >= 9.0 {
            p.text(pos2(numbers_x, y), Align2::RIGHT_CENTER, format!("{}", mark.abs()), regular(8.0), t.text_dim);
            last = y;
        }
    }
    let pos = openpt_model::fader_db_to_pos(db);
    let y = slot.max.y - slot.height() * pos;
    let cap = Rect::from_center_size(pos2(slot.center().x, y), vec2(cap_w, FADER_CAP_H));
    p.rect_filled(cap.translate(vec2(0.0, 2.0)).expand(1.0), 4.0, Color32::from_black_alpha(80));
    let g = Color32::from_gray;
    p.rect_filled(cap, 3.0, g(34));
    let face = cap.shrink(1.0);
    rounded_vgradient(p, face, 2.0, g(236), g(104));
    vgradient(p, Rect::from_x_y_ranges(face.x_range(), y + 1.0..=face.max.y - 2.0), g(214), g(150));
    p.hline(face.min.x + 2.0..=face.max.x - 2.0, face.min.y + 1.5, Stroke::new(1.0, Color32::from_white_alpha(110)));
    p.hline(face.x_range(), y, Stroke::new(2.0, g(30)));
    for (x, dx) in [(cap.min.x, 2.5), (cap.max.x, -2.5)] {
        p.add(egui::Shape::convex_polygon(vec![pos2(x, y - 3.0), pos2(x + dx, y), pos2(x, y + 3.0)], g(34), Stroke::NONE));
    }
    if resp.double_clicked() || (resp.clicked() && ui.input(|i| i.modifiers.alt)) {
        return Some(0.0);
    }
    if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let dy = resp.drag_delta().y / slot.height() * if fine { 0.1 } else { 1.0 };
        let np = (pos - dy).clamp(0.0, 1.0);
        return Some(openpt_model::fader_pos_to_db(np).clamp(-144.0, 12.0));
    }
    None
}

pub fn db_text(db: f32) -> String {
    if db <= -143.9 { "-inf".into() } else { format!("{db:.1}") }
}

pub struct PannerSpeaker {
    pub label: &'static str,
    pub x: f32,
    pub y: f32,
    pub height: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PannerEdit {
    Position(f32, f32),
    Divergence(f32),
    Height(f32),
}

pub fn surround_panner(ui: &mut Ui, size: f32, speakers: &[PannerSpeaker], puck: (f32, f32), divergence: f32, z: Option<f32>) -> Option<PannerEdit> {
    let t = Tokens::current();
    let mut edit = None;
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let half = size * 0.5 - 6.0;
    let c = r.center();
    let to_screen = |x: f32, y: f32| pos2(c.x + x * half, c.y - y * half);
    let painter = ui.painter_at(r.expand(1.0));
    painter.rect(r, CornerRadius::same(2), Color32::from_rgb(14, 18, 22), Stroke::new(1.0, Color32::from_rgb(70, 74, 80)), StrokeKind::Inside);
    let grid = Stroke::new(1.0, Color32::from_rgb(38, 44, 50));
    painter.line_segment([to_screen(-1.0, 0.0), to_screen(1.0, 0.0)], grid);
    painter.line_segment([to_screen(0.0, -1.0), to_screen(0.0, 1.0)], grid);
    painter.rect_stroke(Rect::from_two_pos(to_screen(-1.0, -1.0), to_screen(1.0, 1.0)), 0.0, grid, StrokeKind::Middle);
    painter.line_segment([pos2(c.x - 5.0, r.min.y + 2.0), pos2(c.x + 5.0, r.min.y + 2.0)], Stroke::new(2.0, Color32::from_rgb(90, 110, 130)));
    for s in speakers {
        let (x, y) = if s.height { (s.x * 0.55, s.y * 0.55) } else { (s.x, s.y) };
        let p = to_screen(x, y);
        if s.height {
            painter.circle_stroke(p, 2.5, Stroke::new(1.0, Color32::from_rgb(150, 150, 170)));
        } else {
            painter.circle_filled(p, 3.0, Color32::from_rgb(170, 170, 176));
        }
        if size >= 70.0 && !s.height {
            let inward = vec2(-(x.signum() * if x.abs() > 0.5 { 1.0 } else { 0.0 }), if y.abs() > 0.5 { y.signum() } else { 0.0 });
            let lp = p + vec2(inward.x * 9.0, inward.y * 7.0);
            painter.text(lp, Align2::CENTER_CENTER, s.label, regular(7.5), t.text_dim);
        }
    }
    let alt = ui.input(|i| i.modifiers.alt);
    if resp.double_clicked() {
        edit = Some(PannerEdit::Position(0.0, 1.0));
    } else if resp.dragged() && alt {
        let d = (divergence - resp.drag_delta().y * 0.01).clamp(0.0, 1.0);
        edit = Some(PannerEdit::Divergence(d));
    } else if (resp.dragged() || resp.clicked())
        && !alt
        && let Some(p) = resp.interact_pointer_pos()
    {
        let x = ((p.x - c.x) / half).clamp(-1.0, 1.0);
        let y = ((c.y - p.y) / half).clamp(-1.0, 1.0);
        edit = Some(PannerEdit::Position(x, y));
    }
    let (px, py) = match edit {
        Some(PannerEdit::Position(x, y)) => (x, y),
        _ => puck,
    };
    let pp = to_screen(px, py);
    let div = match edit {
        Some(PannerEdit::Divergence(d)) => d,
        _ => divergence,
    };
    if div > 0.01 {
        painter.circle(pp, 5.0 + div * half, Color32::from_rgba_unmultiplied(80, 160, 230, 28), Stroke::new(1.0, Color32::from_rgb(70, 130, 190)));
    }
    painter.circle(pp, 5.0, t.counter_text, Stroke::new(1.0, Color32::BLACK));
    let tip = "Surround pan: drag the puck · double-click: front centre · Alt-drag: divergence";
    let _ = resp.on_hover_text(tip);
    let mut sliders: Vec<(&str, f32, u8)> = vec![("div", div, 0)];
    if let Some(z) = z {
        sliders.push(("z", z, 1));
    }
    for (label, v, kind) in sliders {
        let (sr, sresp) = ui.allocate_exact_size(vec2(size, 11.0), Sense::click_and_drag());
        let track = Rect::from_min_max(pos2(sr.min.x + 16.0, sr.center().y - 2.0), pos2(sr.max.x - 2.0, sr.center().y + 2.0));
        ui.painter().text(pos2(sr.min.x + 1.0, sr.center().y), Align2::LEFT_CENTER, label, regular(8.0), t.text_dim);
        ui.painter().rect_filled(track, 1.0, Color32::from_rgb(24, 26, 30));
        ui.painter().rect_filled(
            Rect::from_min_max(track.min, pos2(track.min.x + track.width() * v.clamp(0.0, 1.0), track.max.y)),
            1.0,
            t.counter_text,
        );
        if (sresp.dragged() || sresp.clicked())
            && let Some(p) = sresp.interact_pointer_pos()
        {
            let nv = ((p.x - track.min.x) / track.width().max(1.0)).clamp(0.0, 1.0);
            edit = Some(if kind == 0 { PannerEdit::Divergence(nv) } else { PannerEdit::Height(nv) });
        }
        if sresp.double_clicked() {
            edit = Some(if kind == 0 { PannerEdit::Divergence(0.0) } else { PannerEdit::Height(0.0) });
        }
        let _ = sresp.on_hover_text(if kind == 0 { "Divergence (0 = point source)" } else { "Elevation (height speakers)" });
    }
    edit
}

pub fn multi_meter(ui: &Ui, r: Rect, levels: &[f32], labels: &[String], clip: bool) {
    let n = levels.len().max(1);
    let label_h = 16.0;
    let mr = Rect::from_min_max(r.min, pos2(r.max.x, r.max.y - label_h));
    let gap = 1.0;
    let w = ((mr.width() - gap * (n as f32 - 1.0)) / n as f32).max(1.0);
    for (i, lv) in levels.iter().enumerate() {
        let x = mr.min.x + i as f32 * (w + gap);
        let br = Rect::from_min_size(pos2(x, mr.min.y), vec2(w, mr.height()));
        meter(ui, br, *lv, *lv, clip);
        if let Some(l) = labels.get(i) {
            let galley = ui.painter().layout_no_wrap(l.clone(), regular(7.0), Tokens::current().text_dim);
            let gw = galley.size().x;
            let gh = galley.size().y;
            let pos = pos2(br.center().x - gh * 0.5, mr.max.y + 2.0 + gw.min(label_h - 2.0));
            ui.painter().add(egui::epaint::TextShape::new(pos, galley, Tokens::current().text_dim).with_angle(-std::f32::consts::FRAC_PI_2));
        }
    }
}

pub fn checkbox(ui: &mut Ui, on: &mut bool, label: impl Into<egui::WidgetText>) -> Response {
    let text: egui::WidgetText = label.into();
    let side = 13.0;
    let gap = ui.spacing().icon_spacing;
    let galley = text.into_galley(ui, None, ui.available_width() - side - gap, egui::TextStyle::Button);
    let size = egui::vec2(side + gap + galley.size().x, galley.size().y.max(side).max(ui.spacing().interact_size.y));
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let label_text = galley.text().to_string();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, label_text.clone()));
    if ui.is_rect_visible(rect) {
        let t = Tokens::current();
        let enabled = ui.is_enabled();
        let b = egui::Rect::from_min_size(egui::pos2(rect.min.x, rect.center().y - side * 0.5), egui::vec2(side, side));
        let rim = if resp.hovered() { t.text_dim } else { t.border_light };
        ui.painter().rect(b, 1.5, t.field_bg, egui::Stroke::new(1.0, rim), egui::StrokeKind::Inside);
        if *on {
            let c = if enabled { t.check } else { t.text_dim };
            let p = |x: f32, y: f32| egui::pos2(b.min.x + x * side, b.min.y + y * side);
            ui.painter().line(vec![p(0.2, 0.52), p(0.42, 0.74), p(0.82, 0.24)], egui::Stroke::new(2.0, c));
        }
        let color = if enabled { t.text } else { t.text_dim };
        ui.painter().galley(egui::pos2(b.max.x + gap, rect.center().y - galley.size().y * 0.5), galley, color);
    }
    if drag_toggle(ui, &resp, on) {
        resp.mark_changed();
    }
    resp
}

fn drag_toggle(ui: &Ui, resp: &Response, on: &mut bool) -> bool {
    let ctx = ui.ctx();
    let key = egui::Id::new("openpt-checkbox-drag-target");
    if !ctx.input(|i| i.pointer.primary_down()) {
        ctx.data_mut(|d| d.remove::<bool>(key));
    }
    if resp.drag_started_by(egui::PointerButton::Primary) {
        *on = !*on;
        ctx.data_mut(|d| d.insert_temp(key, *on));
        return true;
    }
    if resp.clicked() {
        *on = !*on;
        return true;
    }
    let Some(target) = ctx.data(|d| d.get_temp::<bool>(key)) else { return false };
    if ui.rect_contains_pointer(resp.rect) && *on != target {
        *on = target;
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{clip_gain_from_pos, clip_gain_to_pos};
    use egui::{Event, Modifiers, PointerButton, Pos2};
    use egui_kittest::{Harness, kittest::Queryable};

    fn press(h: &mut Harness<'_, [bool; 4]>, pos: Pos2, pressed: bool) {
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }

    fn boxes() -> (Harness<'static, [bool; 4]>, Vec<Pos2>) {
        let mut h = Harness::new_ui_state(
            |ui, s: &mut [bool; 4]| {
                ui.vertical(|ui| {
                    for b in s.iter_mut() {
                        super::checkbox(ui, b, "Box");
                    }
                });
            },
            [false, true, false, false],
        );
        h.run();
        let at = h.get_all_by_label("Box").map(|n| n.rect().center()).collect();
        (h, at)
    }

    #[test]
    fn dragging_across_checkboxes_sets_them_all_like_the_first() {
        let (mut h, at) = boxes();
        assert_eq!(at.len(), 4);
        h.event(Event::PointerMoved(at[0]));
        h.run_steps(1);
        press(&mut h, at[0], true);
        for p in &at[1..3] {
            h.event(Event::PointerMoved(*p));
            h.run_steps(2);
        }
        press(&mut h, at[2], false);
        assert_eq!(*h.state(), [true, true, true, false]);
    }

    #[test]
    fn dragging_from_a_checked_box_unchecks_and_release_ends_it() {
        let (mut h, at) = boxes();
        h.state_mut().copy_from_slice(&[true, true, true, true]);
        h.run();
        h.event(Event::PointerMoved(at[1]));
        h.run_steps(1);
        press(&mut h, at[1], true);
        h.event(Event::PointerMoved(at[2]));
        h.run_steps(2);
        press(&mut h, at[2], false);
        h.event(Event::PointerMoved(at[3]));
        h.run_steps(2);
        assert_eq!(*h.state(), [true, false, false, true]);
    }

    #[test]
    fn dragging_past_a_scroll_area_leaves_hidden_rows_alone() {
        let mut h = Harness::new_ui_state(
            |ui, s: &mut [bool; 4]| {
                egui::ScrollArea::vertical().max_height(30.0).show(ui, |ui| {
                    for b in s.iter_mut() {
                        super::checkbox(ui, b, "Box");
                    }
                });
            },
            [false; 4],
        );
        h.run();
        let at: Vec<Pos2> = h.get_all_by_label("Box").map(|n| n.rect().center()).collect();
        assert_eq!(at.len(), 4);
        h.event(Event::PointerMoved(at[0]));
        h.run_steps(1);
        press(&mut h, at[0], true);
        h.event(Event::PointerMoved(at[3]));
        h.run_steps(2);
        press(&mut h, at[3], false);
        assert_eq!(*h.state(), [true, false, false, false]);
    }

    #[test]
    fn a_click_toggles_one_checkbox() {
        let (mut h, at) = boxes();
        h.event(Event::PointerMoved(at[1]));
        h.run_steps(1);
        press(&mut h, at[1], true);
        press(&mut h, at[1], false);
        assert_eq!(*h.state(), [false, false, false, false]);
    }

    #[test]
    fn clip_gain_taper_spans_minus_144_to_plus_36_and_round_trips() {
        assert_eq!(clip_gain_from_pos(0.0), -144.0);
        assert_eq!(clip_gain_from_pos(1.0), 36.0);
        assert_eq!(clip_gain_from_pos(f32::NAN), -144.0);
        assert_eq!(clip_gain_to_pos(f32::INFINITY), 0.0);
        for db in [-144.0, -90.0, -36.0, -12.5, 0.0, 6.0, 36.0] {
            assert!((clip_gain_from_pos(clip_gain_to_pos(db)) - db).abs() < 0.01, "{db}");
        }
    }
}

#[cfg(test)]
mod selector_visual_regressions {
    use super::*;

    fn scaled_input(size: egui::Vec2, scale: f32) -> egui::RawInput {
        let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
        input
    }

    #[test]
    fn capture_selector_visual_fixtures() {
        let _palette = crate::theme::lock_test_palette();
        let Some(directory) = std::env::var_os("CRAFT_UI_VISUAL_FIXTURES").map(std::path::PathBuf::from) else { return };
        std::fs::create_dir_all(&directory).unwrap();
        for scale in [1.0, 1.5, 2.0] {
            let mut ready = false;
            let mut harness = egui_kittest::Harness::builder().with_size(vec2(320.0, 120.0)).with_pixels_per_point(scale).wgpu().build_ui_state(
                move |ui, regions: &mut serde_json::Map<String, serde_json::Value>| {
                    if !ready {
                        crate::fonts::install(ui.ctx());
                        crate::theme::apply(ui.ctx(), false);
                        ready = true;
                        return;
                    }
                    regions.clear();
                    for (index, label) in ["auto read", "no group", "A very long mixer group name"].into_iter().enumerate() {
                        ui.horizontal(|ui| {
                            for width in [46.0, 74.0, 110.0] {
                                let r = selector_box(ui, width, 16.0, label, Tokens::current().text).rect;
                                regions.insert(format!("selector-{index}-{width}"), serde_json::json!([r.left(), r.top(), r.width(), r.height()]));
                            }
                        });
                    }
                },
                serde_json::Map::new(),
            );
            harness.input_mut().max_texture_side = Some(8192);
            harness.run_steps(4);
            let stem = format!("openpt-selectors-dark-{scale}x");
            harness.render().unwrap().save(directory.join(format!("{stem}.png"))).unwrap();
            std::fs::write(
                directory.join(format!("{stem}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "screen": [320.0, 120.0], "scale": scale, "regions": harness.state()
                }))
                .unwrap(),
            )
            .unwrap();
        }
    }

    fn mixer_app(narrow: bool, mode: openpt_model::AutomationMode) -> crate::SoundApp {
        let mut app = crate::SoundApp::new(openpt_engine::Engine::default(), None, Default::default());
        app.run("track.new", serde_json::json!({"name": "Voice"})).unwrap();
        app.run("track.new", serde_json::json!({"name": "Music"})).unwrap();
        let ids: Vec<_> = app.engine.session().tracks.iter().map(|track| track.id).collect();
        for id in ids {
            app.run("mix.automation_mode", serde_json::json!({"track": id.0, "mode": mode.label()})).unwrap();
        }
        app.ui.narrow_mix = narrow;
        app.ui.show_tracks_list = false;
        app.ui.mix_views = vec!["io".into()];
        app
    }

    #[test]
    fn actual_mixer_keeps_every_active_automation_mode_legible() {
        let _palette = crate::theme::lock_test_palette();
        for narrow in [false, true] {
            for mode in openpt_model::AutomationMode::ALL {
                for scale in [1.0, 1.5, 2.0] {
                    let ctx = egui::Context::default();
                    crate::fonts::install(&ctx);
                    crate::theme::apply(&ctx, false);
                    let mut app = mixer_app(narrow, mode);
                    let mut output = ctx.run_ui(scaled_input(vec2(320.0, 640.0), scale), |ui| crate::mix_window::show(&mut app, ui));
                    output.textures_delta.clear();
                    assert_eq!(ctx.content_rect().size(), vec2(320.0, 640.0));
                    assert_eq!(ctx.pixels_per_point(), scale);
                    let long = format!("auto {}", mode.label());
                    let short = if mode == openpt_model::AutomationMode::TouchLatch { "t/l" } else { mode.label() };
                    let texts: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(text)
                                if text.galley.job.text == long || text.galley.job.text == mode.label() || text.galley.job.text == short =>
                            {
                                Some(text)
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(texts.len(), 2, "both tracks paint their automation mode: {mode:?}");
                    for text in texts {
                        assert_eq!(text.galley.rows.len(), 1, "{mode:?} narrow={narrow} at {scale}x: automation text wraps");
                        assert!(
                            !text.galley.elided,
                            "{mode:?} narrow={narrow} at {scale}x: the active mode must remain distinguishable without hovering"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn capture_actual_mixer_visual_fixtures() {
        let _palette = crate::theme::lock_test_palette();
        let Some(directory) = std::env::var_os("CRAFT_UI_VISUAL_FIXTURES").map(std::path::PathBuf::from) else { return };
        std::fs::create_dir_all(&directory).unwrap();
        for mode in openpt_model::AutomationMode::ALL {
            for narrow in [false, true] {
                for scale in [1.0, 1.5, 2.0] {
                    let mut ready = false;
                    let mut harness =
                        egui_kittest::Harness::builder().with_size(vec2(320.0, 640.0)).with_pixels_per_point(scale).wgpu().build_ui_state(
                            move |ui, app: &mut crate::SoundApp| {
                                if !ready {
                                    crate::fonts::install(ui.ctx());
                                    crate::theme::apply(ui.ctx(), false);
                                    ready = true;
                                    return;
                                }
                                crate::mix_window::show(app, ui);
                            },
                            mixer_app(narrow, mode),
                        );
                    harness.input_mut().max_texture_side = Some(8192);
                    harness.run_steps(4);
                    let stem = format!("openpt-mixer-narrow-{narrow}-{}-{scale}x", mode.label().replace('/', "-"));
                    harness.render().unwrap().save(directory.join(format!("{stem}.png"))).unwrap();
                    let strips = harness
                        .output()
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Rect(rect)
                                if (rect.rect.width() - crate::mix_window::strip_width(narrow)).abs() < 0.1 && rect.rect.height() >= 560.0 =>
                            {
                                Some(rect.rect)
                            }
                            _ => None,
                        })
                        .fold(Rect::NOTHING, Rect::union);
                    assert!(strips.is_finite());
                    std::fs::write(
                        directory.join(format!("{stem}.json")),
                        serde_json::to_vec_pretty(&serde_json::json!({
                            "screen": [320.0, 640.0], "scale": scale, "mode": mode.label(), "narrow": narrow,
                            "regions": {"mixer-selectors": [strips.left(), strips.top(), strips.width(), 120.0]}
                        }))
                        .unwrap(),
                    )
                    .unwrap();
                }
            }
        }
    }

    #[test]
    fn compact_mixer_selectors_keep_labels_on_one_visible_line() {
        let _palette = crate::theme::lock_test_palette();
        for width in [46.0, 74.0, 110.0] {
            for label in ["auto read", "no group", "A very long mixer group name"] {
                for scale in [1.0, 1.5, 2.0] {
                    let ctx = egui::Context::default();
                    crate::fonts::install(&ctx);
                    crate::theme::apply(&ctx, false);
                    let mut rect = Rect::NOTHING;
                    let mut output = ctx.run_ui(scaled_input(vec2(320.0, 120.0), scale), |ui| {
                        rect = selector_box(ui, width, 16.0, label, Tokens::current().text).rect;
                    });
                    output.textures_delta.clear();
                    assert_eq!(ctx.content_rect().size(), vec2(320.0, 120.0));
                    assert_eq!(ctx.pixels_per_point(), scale);
                    let text = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.job.text == label => Some(text),
                            _ => None,
                        })
                        .expect("selector label is painted");
                    assert_eq!(text.galley.rows.len(), 1, "{label:?} at {width} must not wrap into clipped rows");
                    let bounds = text.visual_bounding_rect();
                    assert!(bounds.top() >= rect.top() && bounds.bottom() <= rect.bottom(), "{label:?}: {bounds:?} outside {rect:?}");
                    assert!(
                        bounds.left() >= rect.left() && bounds.right() <= rect.right() - 16.0 * 0.8,
                        "text must leave room for the dropdown arrow"
                    );
                }
            }
        }
    }
}
