use super::*;
use openpt_dsp::plugins::open_sat::{self as dsp, BANDS};

pub const W: f32 = 940.0;
const H: f32 = 540.0;

#[derive(Default)]
struct State {
    selected: Option<usize>,
    drag: Option<(usize, Pos2, f32, f32)>,
    smooth: Smoothed,
}

fn k(b: usize, key: usize) -> &'static str {
    dsp::band_param(b, key)
}

fn used(cx: &Ctx, b: usize) -> bool {
    cx.on(k(b, dsp::K_ON))
}

fn type_color(t: usize) -> Color32 {
    match t {
        dsp::TAPE => Color32::from_rgb(236, 206, 84),
        dsp::TUBE => Color32::from_rgb(244, 140, 82),
        dsp::PREAMP => Color32::from_rgb(90, 200, 250),
        _ => Color32::from_rgb(184, 164, 255),
    }
}

pub fn show(ui: &mut Ui, cx: &mut Ctx) {
    let (body, _) = plate(ui, vec2(W, H), "Sat");
    with_state::<State, _>(cx.id, |st| {
        let g = Rect::from_min_size(body.min + vec2(12.0, 10.0), vec2(W - 24.0, 340.0));
        graph(ui, cx, st, g);
        let strip = Rect::from_min_max(pos2(body.min.x + 12.0, g.max.y + 10.0), body.max - vec2(12.0, 8.0));
        controls(ui, cx, st, strip);
    });
}

fn graph(ui: &mut Ui, cx: &mut Ctx, st: &mut State, g: Rect) {
    let resp = ui.interact(g, cx.id.with("graph"), Sense::click_and_drag());
    let p = ui.painter_at(g.expand(2.0));
    display(&p, g);
    let fa = FreqAxis { r: g, lo: 20.0, hi: 20_000.0 };
    let sa = DbAxis { r: g, lo: -96.0, hi: 6.0 };
    if let Some(scope) = cx.scope.clone() {
        let cols = (g.width() / 2.0) as usize;
        st.smooth.update(&scope, &fa, cols, false, true);
        spectrum(&p, &fa, &sa, &st.smooth.post, Color32::from_rgba_unmultiplied(74, 47, 143, 90), Color32::from_rgba_unmultiplied(184, 164, 255, 90));
    }
    fa.grid(&p);
    for pct in [25.0f32, 50.0, 75.0, 100.0] {
        let y = g.max.y - 8.0 - (g.height() - 20.0) * pct / 100.0;
        p.hline(g.x_range(), y, Stroke::new(1.0, GRID));
        p.text(pos2(g.max.x - 3.0, y), Align2::RIGHT_CENTER, format!("{pct:.0}%"), regular(9.0), DIM);
    }
    let rate = cx.rate();
    let y_of = |d: f32| g.max.y - 8.0 - (g.height() - 20.0) * d.clamp(0.0, 100.0) / 100.0;
    let drive_of = |y: f32| ((g.max.y - 8.0 - y) / (g.height() - 20.0) * 100.0).clamp(0.0, 100.0);
    let node = |cx: &Ctx, b: usize| pos2(fa.x(cx.get(k(b, dsp::K_FREQ))), y_of(cx.get(k(b, dsp::K_DRIVE))));
    let cols = (g.width() / 3.0) as usize;
    for b in (0..BANDS).filter(|&b| used(cx, b)) {
        let (f0, q, shape, drive) = (cx.get(k(b, dsp::K_FREQ)), cx.get(k(b, dsp::K_Q)), cx.idx(k(b, dsp::K_SHAPE)), cx.get(k(b, dsp::K_DRIVE)));
        let col = type_color(cx.idx(k(b, dsp::K_TYPE)));
        let sel = st.selected == Some(b);
        let byp = cx.on(k(b, dsp::K_BYP));
        let top = y_of(drive.max(3.0));
        let base = g.max.y - 8.0;
        let mut mesh = egui::Mesh::default();
        let a = if byp { 14 } else if sel { 80 } else { 46 };
        let fill = Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), a);
        let mut line = Vec::with_capacity(cols + 1);
        let mags: Vec<f32> = (0..=cols).map(|i| dsp::band_magnitude(fa.f(g.min.x + g.width() * i as f32 / cols as f32), f0, q, rate, shape)).collect();
        let peak = mags.iter().copied().fold(1.0e-6, f32::max);
        for (i, mag) in mags.iter().enumerate() {
            let x = g.min.x + g.width() * i as f32 / cols as f32;
            let m = (mag / peak).min(1.0);
            let y = base - (base - top) * m;
            line.push(pos2(x, y));
            mesh.colored_vertex(pos2(x, y), fill);
            mesh.colored_vertex(pos2(x, base), fill.gamma_multiply(0.3));
            if i > 0 {
                let kk = (2 * i) as u32;
                mesh.add_triangle(kk - 2, kk - 1, kk);
                mesh.add_triangle(kk - 1, kk, kk + 1);
            }
        }
        p.add(mesh);
        p.add(egui::Shape::line(line, Stroke::new(if sel { 1.5 } else { 1.0 }, col)));
        let c = node(cx, b);
        p.circle(c, 8.0, if byp { DISPLAY } else { col }, Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { Color32::WHITE } else { col }));
        p.text(c, Align2::CENTER_CENTER, format!("{}", b + 1), bold(9.0), if byp { col } else { Color32::from_rgb(16, 14, 22) });
        let pos = dsp::POSITIONS[cx.idx(k(b, dsp::K_POS)).min(2)];
        p.text(pos2(c.x, c.y - 16.0), Align2::CENTER_CENTER, format!("{} · {pos}", dsp::TYPES[cx.idx(k(b, dsp::K_TYPE)).min(3)]), regular(9.5), col);
    }
    let hit = |cx: &Ctx, at: Pos2| (0..BANDS).filter(|&b| used(cx, b)).find(|&b| node(cx, b).distance(at) < 10.0);
    let hovered = resp.hover_pos().and_then(|at| hit(cx, at));
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if resp.drag_started()
        && let Some(at) = resp.interact_pointer_pos()
    {
        st.drag = hit(cx, at).map(|b| (b, at, cx.get(k(b, dsp::K_FREQ)), cx.get(k(b, dsp::K_DRIVE))));
        if let Some(d) = st.drag {
            st.selected = Some(d.0);
        }
    }
    if resp.dragged()
        && let (Some((b, start, f0, d0)), Some(at)) = (st.drag, resp.interact_pointer_pos())
    {
        let fine = if ui.input(|i| i.modifiers.shift) { 0.15 } else { 1.0 };
        cx.set(k(b, dsp::K_FREQ), fa.f(fa.x(f0) + (at.x - start.x) * fine).clamp(20.0, 20_000.0));
        cx.set(k(b, dsp::K_DRIVE), (d0 + (drive_of(start.y + (at.y - start.y) * fine) - drive_of(start.y))).clamp(0.0, 100.0));
    }
    if resp.drag_stopped() {
        st.drag = None;
    }
    if resp.clicked()
        && let Some(at) = resp.interact_pointer_pos()
    {
        st.selected = hit(cx, at);
    }
    if resp.secondary_clicked()
        && let Some(b) = resp.interact_pointer_pos().and_then(|at| hit(cx, at))
    {
        cx.set(k(b, dsp::K_ON), 0.0);
        if st.selected == Some(b) {
            st.selected = None;
        }
    }
    if resp.double_clicked()
        && let Some(at) = resp.interact_pointer_pos()
        && hit(cx, at).is_none()
        && let Some(b) = (0..BANDS).find(|&b| !used(cx, b))
    {
        let t = (at.x - g.min.x) / g.width();
        let shape = if t < 0.1 {
            dsp::LOW_SHELF
        } else if t > 0.9 {
            dsp::HIGH_SHELF
        } else {
            dsp::BELL
        };
        for (key, v) in [(dsp::K_FREQ, fa.f(at.x)), (dsp::K_DRIVE, drive_of(at.y)), (dsp::K_Q, 1.2), (dsp::K_SHAPE, shape as f32), (dsp::K_BYP, 0.0), (dsp::K_ON, 1.0)] {
            cx.set(k(b, key), v);
        }
        st.selected = Some(b);
    }
    if let Some(b) = hovered.or(st.drag.map(|d| d.0)) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        if wheel != 0.0 {
            cx.set(k(b, dsp::K_Q), (cx.get(k(b, dsp::K_Q)) * 2f32.powf(wheel * 0.004)).clamp(0.3, 12.0));
        }
    }
}

fn controls(ui: &mut Ui, cx: &mut Ctx, st: &mut State, strip: Rect) {
    let p = &ui.painter().clone();
    p.rect_filled(strip, 3.0, PLATE_HI);
    let cell = vec2(74.0, 92.0);
    let row = strip.min.y + 8.0;
    let mut x = strip.min.x + 8.0;
    if let Some(b) = st.selected.filter(|&b| used(cx, b)) {
        let kind = cx.idx(k(b, dsp::K_TYPE));
        let col = type_color(kind);
        p.text(pos2(x, row + 2.0), Align2::LEFT_TOP, format!("BAND {}", b + 1), bold(12.0), col);
        for (i, t) in dsp::TYPES.iter().enumerate() {
            let r = Rect::from_min_size(pos2(x + (i % 2) as f32 * 76.0, row + 22.0 + (i / 2) as f32 * 26.0), vec2(72.0, 22.0));
            if button(ui, r, &t.to_uppercase(), i == kind, type_color(i)).clicked() {
                cx.set(k(b, dsp::K_TYPE), i as f32);
            }
        }
        let cr = Rect::from_min_size(pos2(x, row + 78.0), vec2(148.0, 0.0).max(vec2(148.0, (strip.max.y - row - 86.0).max(30.0))));
        display(p, cr);
        let drive = cx.get(k(b, dsp::K_DRIVE));
        let gain = 10f32.powf(30.0 * drive * 0.01 / 20.0).min(8.0);
        let pts: Vec<Pos2> = (0..=60)
            .map(|i| {
                let xin = -1.0 + 2.0 * i as f32 / 60.0;
                let y = dsp::shape(kind, xin * gain) / gain.max(1.0);
                pos2(cr.min.x + cr.width() * i as f32 / 60.0, cr.center().y - y.clamp(-1.2, 1.2) * cr.height() * 0.4)
            })
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.5, col)));
        x += 162.0;
        for (key, label) in [(dsp::K_DRIVE, "Drive"), (dsp::K_FREQ, "Freq"), (dsp::K_Q, "Q")] {
            knob(ui, cx, Rect::from_min_size(pos2(x, row), cell), k(b, key), label, col);
            x += cell.x;
        }
        for (i, (key, labels)) in [(dsp::K_SHAPE, dsp::SHAPES), (dsp::K_POS, dsp::POSITIONS)].into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(x + 6.0, row + 8.0 + i as f32 * 26.0), vec2(110.0, 20.0));
            if let Some(v) = choice(ui, r, (cx.id, b, key), labels, cx.idx(k(b, key))) {
                cx.set(k(b, key), v as f32);
            }
        }
        let byp = cx.on(k(b, dsp::K_BYP));
        if button(ui, Rect::from_min_size(pos2(x + 6.0, row + 60.0), vec2(52.0, 20.0)), "BYPASS", byp, GR).clicked() {
            cx.set(k(b, dsp::K_BYP), if byp { 0.0 } else { 1.0 });
        }
        if button(ui, Rect::from_min_size(pos2(x + 64.0, row + 60.0), vec2(52.0, 20.0)), "REMOVE", false, GR).clicked() {
            cx.set(k(b, dsp::K_ON), 0.0);
            st.selected = None;
        }
    } else {
        p.text(pos2(x, row + 30.0), Align2::LEFT_CENTER, "Double-click the display to add a band (near the ends: a shelf).", regular(12.0), DIM);
        p.text(pos2(x, row + 50.0), Align2::LEFT_CENTER, "Drag a dot: frequency and drive. Wheel: width. Right-click: remove.", regular(11.0), DIM);
    }
    let gx = strip.max.x - 8.0 - cell.x - 100.0;
    let matching = cx.on("gainmatch");
    if button(ui, Rect::from_min_size(pos2(gx, row + 14.0), vec2(92.0, 22.0)), "GAIN MATCH", matching, READOUT).clicked() {
        cx.set("gainmatch", if matching { 0.0 } else { 1.0 });
    }
    if matching {
        let m = cx.scope.as_ref().map_or(0.0, |s| s.read(|d| d.readout));
        readout(p, Rect::from_min_size(pos2(gx, row + 42.0), vec2(92.0, 16.0)), &format!("{m:+.1} dB"));
    } else {
        knob(ui, cx, Rect::from_min_size(pos2(gx + 100.0, row), cell), "gain", "Output", ACCENT);
    }
}
