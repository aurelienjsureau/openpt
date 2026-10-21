use super::*;
use openpt_dsp::plugins::open_comp::{self as dsp, STYLES};

pub const W: f32 = 960.0;
const H: f32 = 470.0;

#[derive(Default)]
struct State {
    drag: Option<(f32, f32)>,
    in_peak: f32,
    out_peak: f32,
}

pub fn show(ui: &mut Ui, cx: &mut Ctx) {
    let (body, _) = plate(ui, vec2(W, H), "Comp");
    with_state::<State, _>(cx.id, |st| {
        let hist = Rect::from_min_size(body.min + vec2(12.0, 10.0), vec2(620.0, 280.0));
        let curve = Rect::from_min_size(pos2(hist.max.x + 12.0, hist.min.y), vec2(220.0, 220.0));
        let meters = Rect::from_min_max(pos2(curve.max.x + 10.0, hist.min.y), pos2(body.max.x - 12.0, hist.max.y));
        history(ui, cx, st, hist);
        transfer(ui, cx, curve);
        meter_bars(ui, cx, st, meters);
        let styles = Rect::from_min_max(pos2(curve.min.x, curve.max.y + 10.0), pos2(curve.max.x, hist.max.y));
        style_buttons(ui, cx, styles);
        let strip = Rect::from_min_max(pos2(body.min.x + 12.0, hist.max.y + 10.0), body.max - vec2(12.0, 8.0));
        controls(ui, cx, strip);
    });
}

fn history(ui: &mut Ui, cx: &mut Ctx, st: &mut State, g: Rect) {
    let resp = ui.interact(g, cx.id.with("history"), Sense::click_and_drag());
    let p = ui.painter_at(g.expand(2.0));
    display(&p, g);
    let da = DbAxis { r: g.shrink2(vec2(0.0, 6.0)), lo: -60.0, hi: 3.0 };
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
            let y = da.y(v.max(-60.0)).clamp(g.min.y, bottom);
            mesh.colored_vertex(pos2(x, y), col);
            mesh.colored_vertex(pos2(x, bottom), col);
            if i > 0 {
                let k = (2 * i) as u32;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k, k + 1);
            }
        }
        gr_pts.push(pos2(x, g.min.y + 4.0 + c.gr.min(30.0) / 30.0 * (g.height() * 0.5)));
    }
    p.add(inp);
    p.add(out);
    if gr_pts.len() > 1 {
        p.add(egui::Shape::line(gr_pts, Stroke::new(2.0, GR)));
    }
    let thr = cx.get("threshold");
    let ty = da.y(thr);
    let near = resp.hover_pos().is_some_and(|h| (h.y - ty).abs() < 6.0);
    p.hline(g.x_range(), ty, Stroke::new(if near || st.drag.is_some() { 2.0 } else { 1.0 }, ACCENT));
    p.text(pos2(g.min.x + 6.0, ty - 3.0), Align2::LEFT_BOTTOM, format!("Threshold {thr:.1} dB"), regular(10.0), ACCENT);
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
        cx.set("threshold", (t0 + d).clamp(-60.0, 0.0));
    }
    if resp.drag_stopped() {
        st.drag = None;
    }
}

fn transfer(ui: &mut Ui, cx: &mut Ctx, r: Rect) {
    let p = ui.painter_at(r.expand(2.0));
    display(&p, r);
    let ax = DbAxis { r, lo: -60.0, hi: 0.0 };
    for d in (-54..=0).step_by(6) {
        let y = ax.y(d as f32);
        let x = r.min.x + (d as f32 + 60.0) / 60.0 * r.width();
        p.hline(r.x_range(), y, Stroke::new(1.0, GRID));
        p.vline(x, r.y_range(), Stroke::new(1.0, GRID));
    }
    p.line_segment([r.left_bottom(), r.right_top()], Stroke::new(1.0, GRID_STRONG));
    let (thr, ratio, range) = (cx.get("threshold"), cx.get("ratio"), cx.get("range"));
    let style = cx.idx("style");
    let knee = cx.get("knee").max(match style {
        dsp::OPTO | dsp::VOCAL => 12.0,
        dsp::MASTERING => 6.0,
        _ => 0.0,
    });
    let ratio = if style == dsp::VOCAL { (ratio * 1.6).min(100.0) } else { ratio };
    let pts: Vec<Pos2> = (0..=120)
        .map(|i| {
            let lvl = -60.0 + 60.0 * i as f32 / 120.0;
            let out = lvl - dsp::static_gr(lvl, thr, ratio, knee, range);
            pos2(r.min.x + (lvl + 60.0) / 60.0 * r.width(), ax.y(out).clamp(r.min.y, r.max.y))
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.0, ACCENT)));
    if let Some(s) = &cx.scope {
        let (lvl, gr) = s.read(|d| {
            let last = d.history[(d.hist_pos + openpt_dsp::scope::HISTORY - 1) % openpt_dsp::scope::HISTORY];
            (last.aux, d.readout)
        });
        if lvl > -60.0 {
            let c = pos2(r.min.x + (lvl.min(0.0) + 60.0) / 60.0 * r.width(), ax.y((lvl - gr).max(-60.0)));
            p.circle_filled(c, 4.0, READOUT);
        }
    }
    let tx = r.min.x + (thr + 60.0) / 60.0 * r.width();
    p.vline(tx, r.y_range(), Stroke::new(1.0, ACCENT_DEEP));
    let ratio_txt = if cx.get("ratio") >= 99.9 { "∞:1".to_string() } else { format!("{:.1}:1", cx.get("ratio")) };
    p.text(r.min + vec2(6.0, 6.0), Align2::LEFT_TOP, ratio_txt, bold(11.0), TEXT);
}

fn meter_bars(ui: &mut Ui, cx: &mut Ctx, st: &mut State, r: Rect) {
    let p = ui.painter_at(r.expand(2.0));
    let last = cx.scope.as_ref().map(|s| s.read(|d| d.history[(d.hist_pos + openpt_dsp::scope::HISTORY - 1) % openpt_dsp::scope::HISTORY]));
    let (i, o, g) = last.map_or((-120.0, -120.0, 0.0), |c| (c.input, c.output, c.gr));
    st.in_peak = st.in_peak.max(i) - 0.3;
    st.out_peak = st.out_peak.max(o) - 0.3;
    let w = (r.width() - 16.0) / 3.0;
    let bar = |k: usize| Rect::from_min_size(pos2(r.min.x + k as f32 * (w + 8.0), r.min.y + 14.0), vec2(w, r.height() - 30.0));
    let level = |b: Rect, db: f32, peak: f32, col: Color32| {
        display(&p, b);
        let t = ((db + 60.0) / 63.0).clamp(0.0, 1.0);
        p.rect_filled(Rect::from_min_max(pos2(b.min.x + 1.0, b.max.y - t * b.height()), b.max - vec2(1.0, 0.0)), 0.0, col);
        let tp = ((peak + 60.0) / 63.0).clamp(0.0, 1.0);
        p.hline(b.x_range(), b.max.y - tp * b.height(), Stroke::new(1.0, TEXT));
    };
    let (bi, bg, bo) = (bar(0), bar(1), bar(2));
    level(bi, i, st.in_peak, Color32::from_rgb(120, 118, 140));
    display(&p, bg);
    let t = (g / 30.0).clamp(0.0, 1.0);
    p.rect_filled(Rect::from_min_max(bg.min + vec2(1.0, 0.0), pos2(bg.max.x - 1.0, bg.min.y + t * bg.height())), 0.0, GR);
    level(bo, o, st.out_peak, ACCENT);
    for (b, t) in [(bi, "IN"), (bg, "GR"), (bo, "OUT")] {
        caption(&p, pos2(b.center().x, r.min.y + 6.0), t);
    }
    readout(&p, Rect::from_min_size(pos2(bg.min.x - 8.0, r.max.y - 14.0), vec2(bg.width() + 16.0, 14.0)), &format!("{:.1}", g));
}

fn style_buttons(ui: &mut Ui, cx: &mut Ctx, r: Rect) {
    let cur = cx.idx("style");
    let (cols, rows) = (4, 2);
    let w = (r.width() - (cols - 1) as f32 * 4.0) / cols as f32;
    let h = ((r.height() - 4.0) / rows as f32).min(22.0);
    for (i, s) in STYLES.iter().enumerate() {
        let b = Rect::from_min_size(pos2(r.min.x + (i % cols) as f32 * (w + 4.0), r.min.y + (i / cols) as f32 * (h + 4.0)), vec2(w, h));
        if button(ui, b, s, i == cur, ACCENT).clicked() {
            cx.set("style", i as f32);
        }
    }
}

fn controls(ui: &mut Ui, cx: &mut Ctx, strip: Rect) {
    let p = &ui.painter().clone();
    p.rect_filled(strip, 3.0, PLATE_HI);
    let cell = vec2(64.0, 90.0);
    let row = strip.min.y + 8.0;
    let mut x = strip.min.x + 6.0;
    for (id, label) in [
        ("threshold", "Threshold"),
        ("ratio", "Ratio"),
        ("knee", "Knee"),
        ("range", "Range"),
        ("attack", "Attack"),
        ("release", "Release"),
        ("lookahead", "Lookahead"),
        ("hold", "Hold"),
        ("gain", "Gain"),
        ("mix", "Dry/Wet"),
    ] {
        let r = Rect::from_min_size(pos2(x, row), cell);
        if id == "ratio" {
            let prm = cx.param(id);
            if let Some(prm) = prm {
                let v = cx.get(id);
                knob_with(ui, cx, r, prm, v, label, ACCENT, &|v| if v >= 99.9 { "∞:1".into() } else { format!("{v:.1}:1") });
            }
        } else {
            knob(ui, cx, r, id, label, if id == "gain" || id == "mix" { READOUT } else { ACCENT });
        }
        x += cell.x;
    }
    let bx = x + 6.0;
    for (i, (id, label)) in [("auto_release", "AUTO REL"), ("auto_gain", "AUTO GAIN"), ("audition", "AUDITION")].into_iter().enumerate() {
        let on = cx.on(id);
        if button(
            ui,
            Rect::from_min_size(pos2(bx, row + 4.0 + i as f32 * 26.0), vec2(74.0, 22.0)),
            label,
            on,
            if id == "audition" { Color32::from_rgb(222, 196, 52) } else { READOUT },
        )
        .clicked()
        {
            cx.set(id, if on { 0.0 } else { 1.0 });
        }
    }
    let sx = bx + 82.0;
    p.text(pos2(sx, row - 2.0), Align2::LEFT_TOP, "SIDE CHAIN", regular(9.0), DIM);
    let small = vec2(64.0, 90.0);
    knob_with_off(ui, cx, Rect::from_min_size(pos2(sx, row + 6.0), small), "sc_hp", "High Pass", |v| v <= 10.5);
    knob_with_off(ui, cx, Rect::from_min_size(pos2(sx + small.x, row + 6.0), small), "sc_lp", "Low Pass", |v| v >= 19_500.0);
    knob(ui, cx, Rect::from_min_size(pos2(sx + 2.0 * small.x, row + 6.0), small), "link", "Link", ACCENT);
}

fn knob_with_off(ui: &mut Ui, cx: &mut Ctx, r: Rect, id: &str, label: &str, off: impl Fn(f32) -> bool) {
    let Some(prm) = cx.param(id) else { return };
    let v = cx.get(id);
    knob_with(ui, cx, r, prm, v, label, ACCENT, &|v| if off(v) { "Off".into() } else { prm.format(v) });
}
