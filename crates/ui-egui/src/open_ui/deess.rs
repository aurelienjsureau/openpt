use super::*;
use openpt_dsp::plugins::open_ds::{BANDS, MODES};

pub const W: f32 = 840.0;
const H: f32 = 400.0;
const LO: f32 = -60.0;

#[derive(Default)]
pub(super) struct Meters {
    drag: Option<(f32, f32)>,
    in_peak: f32,
    out_peak: f32,
}

pub fn show(ui: &mut Ui, cx: &mut Ctx) {
    let (body, _) = plate(ui, vec2(W, H), "DS");
    with_state::<Meters, _>(cx.id, |st| {
        let hist = Rect::from_min_size(body.min + vec2(12.0, 10.0), vec2(600.0, 220.0));
        let meters = Rect::from_min_max(pos2(hist.max.x + 12.0, hist.min.y), pos2(body.max.x - 12.0, hist.max.y));
        history(ui, cx, st, hist, LO, Some("threshold"), cx.get("range").max(1.0));
        meter_bars(ui, cx, st, meters, LO, cx.get("range").max(1.0));
        let strip = Rect::from_min_max(pos2(body.min.x + 12.0, hist.max.y + 10.0), body.max - vec2(12.0, 8.0));
        let p = &ui.painter().clone();
        p.rect_filled(strip, 3.0, PLATE_HI);
        let col = Rect::from_min_size(strip.min + vec2(8.0, 8.0), vec2(150.0, strip.height() - 16.0));
        p.text(col.min, Align2::LEFT_TOP, t!("MODE"), regular(9.0), DIM);
        let mode = cx.idx("mode");
        for (i, m) in MODES.iter().enumerate() {
            let b = Rect::from_min_size(pos2(col.min.x, col.min.y + 14.0 + i as f32 * 26.0), vec2(col.width(), 22.0));
            if button(ui, b, m, i == mode, ACCENT).clicked() {
                cx.set("mode", i as f32);
            }
        }
        let band = cx.idx("band");
        p.text(pos2(col.min.x, col.min.y + 72.0), Align2::LEFT_TOP, t!("PROCESSING"), regular(9.0), DIM);
        for (i, m) in BANDS.iter().enumerate() {
            let b =
                Rect::from_min_size(pos2(col.min.x + i as f32 * (col.width() / 2.0 + 2.0), col.min.y + 86.0), vec2(col.width() / 2.0 - 2.0, 22.0));
            if button(ui, b, m, i == band, ACCENT).clicked() {
                cx.set("band", i as f32);
            }
        }
        let cell = vec2(78.0, 100.0);
        let mut x = col.max.x + 14.0;
        for (id, label) in
            [("threshold", "Threshold"), ("range", "Range"), ("freq_lo", "Low"), ("freq_hi", "High"), ("lookahead", "Lookahead"), ("link", "Link")]
        {
            knob(ui, cx, Rect::from_min_size(pos2(x, strip.min.y + 12.0), cell), id, label, if id.starts_with("freq") { READOUT } else { ACCENT });
            x += cell.x;
        }
        let on = cx.on("listen");
        let lb = Rect::from_min_size(pos2(x + 6.0, strip.min.y + 26.0), vec2(strip.max.x - x - 18.0, 26.0));
        if button(ui, lb, "LISTEN", on, Color32::from_rgb(222, 196, 52)).on_hover_text(t!("Hear what triggers the de-esser")).clicked() {
            cx.set("listen", if on { 0.0 } else { 1.0 });
        }
    });
}

pub(super) fn history(ui: &mut Ui, cx: &mut Ctx, st: &mut Meters, g: Rect, lo: f32, line: Option<&str>, gr_span: f32) {
    let resp = ui.interact(g, cx.id.with("history"), Sense::click_and_drag());
    let p = ui.painter_at(g.expand(2.0));
    display(&p, g);
    let da = DbAxis { r: g.shrink2(vec2(0.0, 6.0)), lo, hi: 3.0 };
    da.grid(&p, 6.0, true);
    let cols: Vec<openpt_dsp::scope::Column> = cx.scope.as_ref().map(|s| s.read(|d| d.columns_in_order().copied().collect())).unwrap_or_default();
    let n = (g.width() / 1.5) as usize;
    let shown = &cols[cols.len().saturating_sub(n)..];
    let step = g.width() / n.max(1) as f32;
    let x0 = g.max.x - shown.len() as f32 * step;
    let bottom = g.max.y;
    let mut inp = egui::Mesh::default();
    let mut out = egui::Mesh::default();
    let ic = Color32::from_rgba_unmultiplied(120, 118, 140, 90);
    let oc = Color32::from_rgba_unmultiplied(74, 47, 143, 200);
    let mut gr_pts = Vec::with_capacity(shown.len());
    for (i, c) in shown.iter().enumerate() {
        let x = x0 + i as f32 * step;
        for (mesh, v, col) in [(&mut inp, c.input, ic), (&mut out, c.output, oc)] {
            let y = da.y(v.max(lo)).clamp(g.min.y, bottom);
            mesh.colored_vertex(pos2(x, y), col);
            mesh.colored_vertex(pos2(x, bottom), col);
            if i > 0 {
                let k = (2 * i) as u32;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k, k + 1);
            }
        }
        gr_pts.push(pos2(x, g.min.y + 4.0 + (c.gr / gr_span).min(1.0) * (g.height() * 0.4)));
    }
    p.add(inp);
    p.add(out);
    if gr_pts.len() > 1 {
        p.add(egui::Shape::line(gr_pts, Stroke::new(2.0, GR)));
    }
    let Some(id) = line else { return };
    let Some(prm) = cx.param(id) else { return };
    let thr = cx.get(id);
    let ty = da.y(thr);
    let near = resp.hover_pos().is_some_and(|h| (h.y - ty).abs() < 6.0);
    p.hline(g.x_range(), ty, Stroke::new(if near || st.drag.is_some() { 2.0 } else { 1.0 }, ACCENT));
    p.text(pos2(g.min.x + 6.0, ty - 3.0), Align2::LEFT_BOTTOM, format!("{} {thr:.1} dB", prm.name), regular(10.0), ACCENT);
    if near {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if resp.drag_started()
        && let Some(at) = ui.input(|i| i.pointer.press_origin()).or_else(|| resp.interact_pointer_pos())
    {
        st.drag = ((at.y - ty).abs() < 8.0).then_some((at.y, thr));
    }
    if resp.dragged()
        && let (Some((y0, t0)), Some(at)) = (st.drag, resp.interact_pointer_pos())
    {
        let fine = if ui.input(|i| i.modifiers.shift) { 0.15 } else { 1.0 };
        let d = da.db(y0 + (at.y - y0) * fine) - da.db(y0);
        cx.set(id, (t0 + d).clamp(prm.min.max(lo), prm.max));
    }
    if resp.drag_stopped() {
        st.drag = None;
    }
}

pub(super) fn meter_bars(ui: &mut Ui, cx: &mut Ctx, st: &mut Meters, r: Rect, lo: f32, gr_span: f32) {
    let p = ui.painter_at(r.expand(2.0));
    let last = cx.scope.as_ref().map(|s| s.read(|d| d.history[(d.hist_pos + openpt_dsp::scope::HISTORY - 1) % openpt_dsp::scope::HISTORY]));
    let (i, o, g) = last.map_or((-120.0, -120.0, 0.0), |c| (c.input, c.output, c.gr));
    st.in_peak = st.in_peak.max(i) - 0.3;
    st.out_peak = st.out_peak.max(o) - 0.3;
    let w = (r.width() - 16.0) / 3.0;
    let bar = |k: usize| Rect::from_min_size(pos2(r.min.x + k as f32 * (w + 8.0), r.min.y + 14.0), vec2(w, r.height() - 30.0));
    let level = |b: Rect, db: f32, peak: f32, col: Color32| {
        display(&p, b);
        let t = ((db - lo) / (3.0 - lo)).clamp(0.0, 1.0);
        p.rect_filled(Rect::from_min_max(pos2(b.min.x + 1.0, b.max.y - t * b.height()), b.max - vec2(1.0, 0.0)), 0.0, col);
        let tp = ((peak - lo) / (3.0 - lo)).clamp(0.0, 1.0);
        p.hline(b.x_range(), b.max.y - tp * b.height(), Stroke::new(1.0, TEXT));
    };
    let (bi, bg, bo) = (bar(0), bar(1), bar(2));
    level(bi, i, st.in_peak, Color32::from_rgb(120, 118, 140));
    display(&p, bg);
    let t = (g / gr_span).clamp(0.0, 1.0);
    p.rect_filled(Rect::from_min_max(bg.min + vec2(1.0, 0.0), pos2(bg.max.x - 1.0, bg.min.y + t * bg.height())), 0.0, GR);
    level(bo, o, st.out_peak, ACCENT);
    for (b, t) in [(bi, "IN"), (bg, "GR"), (bo, "OUT")] {
        caption(&p, pos2(b.center().x, r.min.y + 6.0), t);
    }
    readout(&p, Rect::from_min_size(pos2(bg.min.x - 8.0, r.max.y - 14.0), vec2(bg.width() + 16.0, 14.0)), &format!("{g:.1}"));
}
