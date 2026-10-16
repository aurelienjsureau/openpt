use super::*;

pub const W: f32 = 880.0;
const H: f32 = 452.0;
const LO: f32 = -60.0;

#[derive(Default)]
struct State {
    drag: Option<(f32, f32)>,
    last_hit: f64,
}

fn note_name(n: f32) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let n = n.round().clamp(0.0, 127.0) as i32;
    format!("{}{} ({n})", NAMES[(n % 12) as usize], n / 12 - 2)
}

pub fn show(ui: &mut Ui, cx: &mut Ctx) {
    let (body, head) = plate(ui, vec2(W, H), "DRT");
    with_state::<State, _>(cx.id, |st| {
        let g = Rect::from_min_size(body.min + vec2(12.0, 10.0), vec2(W - 24.0, 250.0));
        let now = ui.input(|i| i.time);
        scope_view(ui, cx, st, g, now);
        let led = pos2(head.max.x - 70.0, head.center().y);
        let lit = now - st.last_hit < 0.08;
        ui.painter().circle(led, 5.0, if lit { READOUT } else { OFF }, Stroke::new(1.0, EDGE));
        let strip = Rect::from_min_max(pos2(body.min.x + 12.0, g.max.y + 10.0), body.max - vec2(12.0, 8.0));
        controls(ui, cx, strip);
    });
}

fn scope_view(ui: &mut Ui, cx: &mut Ctx, st: &mut State, g: Rect, now: f64) {
    let resp = ui.interact(g, cx.id.with("scope"), Sense::click_and_drag());
    let p = ui.painter_at(g.expand(2.0));
    display(&p, g);
    let da = DbAxis { r: g.shrink2(vec2(0.0, 6.0)), lo: LO, hi: 0.0 };
    da.grid(&p, 6.0, true);
    let (cols, total) = cx.scope.as_ref().map(|s| s.read(|d| (d.columns_in_order().copied().collect::<Vec<_>>(), d.columns))).unwrap_or_default();
    let n = (g.width() / 3.0) as usize;
    let shown = &cols[cols.len().saturating_sub(n)..];
    let step = g.width() / n.max(1) as f32;
    let x0 = g.max.x - shown.len() as f32 * step;
    let fill = Color32::from_rgba_unmultiplied(56, 209, 119, 150);
    let mut mesh = egui::Mesh::default();
    let mut thr = Vec::with_capacity(shown.len());
    for (i, c) in shown.iter().enumerate() {
        let x = x0 + i as f32 * step;
        let y = da.y(c.input.max(LO)).clamp(g.min.y, g.max.y);
        mesh.colored_vertex(pos2(x, y), fill);
        mesh.colored_vertex(pos2(x, g.max.y), fill.gamma_multiply(0.25));
        if i > 0 {
            let k = (2 * i) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
        thr.push(pos2(x, da.y(c.aux.max(LO)).clamp(g.min.y, g.max.y)));
    }
    p.add(mesh);
    if thr.len() > 1 {
        p.add(egui::Shape::line(thr, Stroke::new(1.2, Color32::from_rgb(236, 206, 84))));
    }
    for (i, c) in shown.iter().enumerate() {
        if c.hit.is_finite() {
            p.circle_filled(pos2(x0 + i as f32 * step, da.y(c.hit.max(LO)).clamp(g.min.y + 3.0, g.max.y)), 3.5, Color32::WHITE);
        }
    }
    if shown.iter().rev().take(14).any(|c| c.hit.is_finite()) && total > 0 {
        st.last_hit = now;
    }
    let floor = cx.get("floor");
    let fy = da.y(floor.max(LO));
    let near = resp.hover_pos().is_some_and(|h| (h.y - fy).abs() < 6.0);
    p.add(egui::Shape::dashed_line(&[pos2(g.min.x, fy), pos2(g.max.x, fy)], Stroke::new(if near { 2.0 } else { 1.2 }, GR), 6.0, 4.0));
    p.text(pos2(g.min.x + 6.0, fy - 3.0), Align2::LEFT_BOTTOM, format!("Floor {floor:.1} dB"), regular(10.0), GR);
    if near {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if resp.drag_started()
        && let Some(at) = resp.interact_pointer_pos()
    {
        st.drag = ((at.y - fy).abs() < 8.0).then_some((at.y, floor));
    }
    if resp.dragged()
        && let (Some((y0, f0)), Some(at)) = (st.drag, resp.interact_pointer_pos())
    {
        cx.set("floor", (f0 + da.db(at.y) - da.db(y0)).clamp(-72.0, -12.0));
    }
    if resp.drag_stopped() {
        st.drag = None;
    }
}

fn controls(ui: &mut Ui, cx: &mut Ctx, strip: Rect) {
    let p = &ui.painter().clone();
    p.rect_filled(strip, 3.0, PLATE_HI);
    let cell = vec2(84.0, 96.0);
    let row = strip.min.y + 10.0;
    let mut x = strip.min.x + 10.0;
    p.text(pos2(x, row - 4.0), Align2::LEFT_TOP, "DETECTION", bold(10.0), ACCENT);
    for (id, label) in [("sensitivity", "Sensitivity"), ("retrigger", "Retrigger"), ("floor", "Floor")] {
        knob(ui, cx, Rect::from_min_size(pos2(x, row + 16.0), cell), id, label, READOUT);
        x += cell.x;
    }
    let r = Rect::from_min_size(pos2(x + 4.0, row + 30.0), vec2(116.0, 20.0));
    p.text(pos2(r.min.x, r.min.y - 4.0), Align2::LEFT_BOTTOM, "FOCUS", regular(9.0), DIM);
    if let Some(v) = choice(ui, r, (cx.id, "focus"), openpt_dsp::plugins::open_drt::FOCUS, cx.idx("focus")) {
        cx.set("focus", v as f32);
    }
    let listen = cx.on("listen");
    if button(ui, Rect::from_min_size(pos2(x + 4.0, row + 62.0), vec2(116.0, 22.0)), "LISTEN", listen, Color32::from_rgb(222, 196, 52)).clicked() {
        cx.set("listen", if listen { 0.0 } else { 1.0 });
    }
    x += 140.0;
    p.vline(x - 8.0, strip.min.y + 6.0..=strip.max.y - 6.0, Stroke::new(1.0, EDGE));
    p.text(pos2(x, row - 4.0), Align2::LEFT_TOP, "OUTPUT", bold(10.0), ACCENT);
    knob(ui, cx, Rect::from_min_size(pos2(x, row + 16.0), cell), "triglevel", "Trig Level", ACCENT);
    x += cell.x;
    if let Some(prm) = cx.param("note") {
        let v = cx.get("note");
        knob_with(ui, cx, Rect::from_min_size(pos2(x, row + 16.0), cell), prm, v, "MIDI Note", ACCENT, &|v| note_name(v));
    }
    x += cell.x;
    let r = Rect::from_min_size(pos2(x + 4.0, row + 30.0), vec2(120.0, 20.0));
    p.text(pos2(r.min.x, r.min.y - 4.0), Align2::LEFT_BOTTOM, "SENDS", regular(9.0), DIM);
    if let Some(v) = choice(ui, r, (cx.id, "outmode"), openpt_dsp::plugins::open_drt::OUT_MODES, cx.idx("outmode")) {
        cx.set("outmode", v as f32);
    }
    let dv = cx.on("dynvel");
    if button(ui, Rect::from_min_size(pos2(x + 4.0, row + 62.0), vec2(120.0, 22.0)), "DYNAMIC VELOCITY", dv, READOUT).clicked() {
        cx.set("dynvel", if dv { 0.0 } else { 1.0 });
    }
}
