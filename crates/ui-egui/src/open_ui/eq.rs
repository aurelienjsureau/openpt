use super::*;
use openpt_dsp::plugins::open_eq::{self as dsp, BANDS};

pub const W: f32 = 980.0;
const H: f32 = 560.0;
const RANGES: [f32; 4] = [3.0, 6.0, 12.0, 30.0];

#[derive(Default)]
struct State {
    selected: Option<usize>,
    drag: Option<(usize, Pos2, f32, f32)>,
    range: usize,
    analyzer: usize,
    smooth: Smoothed,
    init: bool,
}

fn k(b: usize, key: usize) -> &'static str {
    dsp::band_param(b, key)
}

pub fn show(ui: &mut Ui, cx: &mut Ctx) {
    let (body, _) = plate(ui, vec2(W, H), "EQ");
    let id = cx.id;
    with_state::<State, _>(id, |st| {
        if !st.init {
            st.init = true;
            st.range = 2;
        }
        let g = Rect::from_min_size(body.min + vec2(12.0, 10.0), vec2(W - 24.0, 390.0));
        graph(ui, cx, st, g);
        let strip = Rect::from_min_max(pos2(body.min.x + 12.0, g.max.y + 10.0), body.max - vec2(12.0, 8.0));
        controls(ui, cx, st, strip);
    });
}

fn bands(cx: &Ctx) -> Vec<dsp::BandSettings> {
    (0..BANDS).map(|b| dsp::band_from(|id| cx.get(id), b)).collect()
}

fn used(cx: &Ctx, b: usize) -> bool {
    cx.on(k(b, dsp::K_USED))
}

fn graph(ui: &mut Ui, cx: &mut Ctx, st: &mut State, g: Rect) {
    let resp = ui.interact(g, cx.id.with("graph"), Sense::click_and_drag());
    let p = ui.painter_at(g.expand(2.0));
    display(&p, g);
    let fa = FreqAxis { r: g, lo: 10.0, hi: 30_000.0 };
    let range = RANGES[st.range.min(3)];
    let da = DbAxis { r: g.shrink2(vec2(0.0, 10.0)), lo: -range, hi: range };
    let sa = DbAxis { r: g, lo: -96.0, hi: 6.0 };
    let fs = cx.rate() as f64;
    if let Some(scope) = cx.scope.clone()
        && st.analyzer < 3
    {
        let cols = (g.width() / 2.0) as usize;
        st.smooth.update(&scope, &fa, cols, st.analyzer != 2, st.analyzer != 1);
        spectrum(&p, &fa, &sa, &st.smooth.pre, Color32::TRANSPARENT, Color32::from_rgba_unmultiplied(150, 150, 170, 90));
        spectrum(
            &p,
            &fa,
            &sa,
            &st.smooth.post,
            Color32::from_rgba_unmultiplied(74, 47, 143, 110),
            Color32::from_rgba_unmultiplied(184, 164, 255, 120),
        );
    }
    fa.grid(&p);
    da.grid(
        &p,
        if range <= 6.0 {
            1.0
        } else if range <= 12.0 {
            3.0
        } else {
            6.0
        },
        true,
    );
    let bs = bands(cx);
    let live: Vec<f32> = cx.scope.as_ref().map(|s| s.read(|d| d.bands[..BANDS].to_vec())).unwrap_or_else(|| vec![0.0; BANDS]);
    let cols = (g.width() / 3.0) as usize;
    let xs: Vec<f32> = (0..=cols).map(|i| g.min.x + g.width() * i as f32 / cols as f32).collect();
    let mut sum = vec![0.0f32; xs.len()];
    let mut sum_dyn = vec![0.0f32; xs.len()];
    let mut any_dyn = false;
    for (b, s) in bs.iter().enumerate() {
        if !used(cx, b) {
            continue;
        }
        let d = dsp::band_design(s, 0.0, fs);
        let col = band_color(b);
        let ys: Vec<f32> = xs.iter().map(|&x| d.mag_db_at(fa.f(x) as f64, fs) as f32).collect();
        if s.active {
            for (a, y) in sum.iter_mut().zip(&ys) {
                *a += y;
            }
            let dyn_d = if s.dynamic { Some(dsp::band_design(s, live[b], fs)) } else { None };
            for (i, a) in sum_dyn.iter_mut().enumerate() {
                *a += dyn_d.as_ref().map_or(ys[i], |dd| dd.mag_db_at(fa.f(xs[i]) as f64, fs) as f32);
            }
            any_dyn |= s.dynamic;
        }
        let zero = da.y(0.0);
        let sel = st.selected == Some(b);
        let mut mesh = egui::Mesh::default();
        let a = if sel { 70 } else { 34 };
        let fill = Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), if s.active { a } else { 12 });
        for (i, (&x, &y)) in xs.iter().zip(&ys).enumerate() {
            let yy = da.y(y.clamp(-range * 1.6, range * 1.6)).clamp(g.min.y, g.max.y);
            mesh.colored_vertex(pos2(x, yy), fill);
            mesh.colored_vertex(pos2(x, zero), fill);
            if i > 0 {
                let k = (2 * i) as u32;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k, k + 1);
            }
        }
        p.add(mesh);
        if sel {
            let pts: Vec<Pos2> = xs.iter().zip(&ys).map(|(&x, &y)| pos2(x, da.y(y).clamp(g.min.y, g.max.y))).collect();
            p.add(egui::Shape::line(pts, Stroke::new(1.0, col)));
        }
    }
    if any_dyn {
        let pts: Vec<Pos2> = xs.iter().zip(&sum_dyn).map(|(&x, &y)| pos2(x, da.y(y).clamp(g.min.y, g.max.y))).collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.2, READOUT)));
    }
    let pts: Vec<Pos2> = xs.iter().zip(&sum).map(|(&x, &y)| pos2(x, da.y(y).clamp(g.min.y, g.max.y))).collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.0, Color32::from_rgb(236, 232, 255))));
    let node = |b: usize| -> Pos2 {
        let s = &bs[b];
        let y = if dsp::has_gain(s.shape) {
            da.y(if s.shape == dsp::TILT_SHELF || s.shape == dsp::FLAT_TILT { s.gain * 0.5 } else { s.gain })
        } else {
            da.y(0.0)
        };
        pos2(fa.x(s.freq), y.clamp(g.min.y + 6.0, g.max.y - 6.0))
    };
    let hover = resp.hover_pos();
    let hit = |cx: &Ctx, at: Pos2| {
        (0..BANDS)
            .filter(|&b| used(cx, b))
            .map(|b| (b, node(b).distance(at)))
            .filter(|(_, d)| *d < 16.0)
            .min_by(|a, c| a.1.total_cmp(&c.1))
            .map(|(b, _)| b)
    };
    let hovered = hover.and_then(|at| hit(cx, at));
    for b in (0..BANDS).filter(|&b| used(cx, b)) {
        let c = node(b);
        let col = band_color(b);
        let s = &bs[b];
        let sel = st.selected == Some(b);
        if s.active {
            p.circle(c, 8.0, col, Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { Color32::WHITE } else { EDGE }));
        } else {
            p.circle(c, 8.0, DISPLAY, Stroke::new(1.5, col));
        }
        if hovered == Some(b) && !sel {
            p.circle_stroke(c, 11.0, Stroke::new(1.0, col));
        }
        p.text(c, Align2::CENTER_CENTER, format!("{}", b + 1), bold(9.0), if s.active { Color32::from_rgb(16, 14, 22) } else { col });
        if s.dynamic && live[b].abs() > 0.05 {
            let y2 = da.y(s.gain + live[b]).clamp(g.min.y, g.max.y);
            p.line_segment([pos2(c.x + 12.0, c.y), pos2(c.x + 12.0, y2)], Stroke::new(3.0, READOUT));
        }
    }
    if let Some(b) = st.drag.map(|d| d.0).or(hovered) {
        let s = &bs[b];
        let f = if s.freq >= 1000.0 { format!("{:.2} kHz", s.freq / 1000.0) } else { format!("{:.0} Hz", s.freq) };
        let txt = if dsp::has_gain(s.shape) { format!("{f}  {:+.1} dB  Q {:.2}", s.gain, s.q) } else { format!("{f}  Q {:.2}", s.q) };
        let c = node(b);
        let r = Rect::from_center_size(pos2(c.x.clamp(g.min.x + 70.0, g.max.x - 70.0), (c.y - 22.0).max(g.min.y + 10.0)), vec2(130.0, 16.0));
        readout(&p, r, &txt);
    }
    let shift = ui.input(|i| i.modifiers.shift);
    if resp.drag_started()
        && let Some(at) = ui.input(|i| i.pointer.press_origin()).or_else(|| resp.interact_pointer_pos())
    {
        if let Some(b) = hit(cx, at) {
            st.selected = Some(b);
            let s = &bs[b];
            st.drag = Some((b, at, s.freq, if dsp::is_cut(s.shape) { s.q } else { s.gain }));
        } else {
            st.drag = None;
        }
    }
    if resp.dragged()
        && let (Some((b, start, f0, v0)), Some(at)) = (st.drag, resp.interact_pointer_pos())
    {
        let fine = if shift { 0.15 } else { 1.0 };
        let s = bs[b];
        let f = fa.f(fa.x(f0) + (at.x - start.x) * fine).clamp(10.0, 30_000.0);
        cx.set(k(b, dsp::K_FREQ), f);
        let ddb = da.db(start.y + (at.y - start.y) * fine) - da.db(start.y);
        if dsp::is_cut(s.shape) {
            cx.set(k(b, dsp::K_Q), (v0 * 10f32.powf(ddb / 20.0)).clamp(0.025, 40.0));
        } else if dsp::has_gain(s.shape) {
            let m = if s.shape == dsp::TILT_SHELF || s.shape == dsp::FLAT_TILT { 2.0 } else { 1.0 };
            cx.set(k(b, dsp::K_GAIN), (v0 + ddb * m).clamp(-30.0, 30.0));
        }
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
        remove(cx, st, b);
    }
    if resp.double_clicked()
        && let Some(at) = resp.interact_pointer_pos()
        && hit(cx, at).is_none()
    {
        add(cx, st, &fa, &da, at);
    }
    if let Some(b) = st.drag.map(|d| d.0).or(hovered) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        if wheel != 0.0 && bs[b].shape != dsp::FLAT_TILT {
            cx.set(k(b, dsp::K_Q), (bs[b].q * 2f32.powf(wheel * 0.004)).clamp(0.025, 40.0));
        }
    }
    if hovered.is_some() || st.drag.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
}

fn add(cx: &mut Ctx, st: &mut State, fa: &FreqAxis, da: &DbAxis, at: Pos2) {
    let Some(b) = (0..BANDS).find(|&b| !used(cx, b)) else { return };
    let t = (at.x - fa.r.min.x) / fa.r.width();
    let shape = if t < 0.05 {
        dsp::LOW_CUT
    } else if t < 0.15 {
        dsp::LOW_SHELF
    } else if t > 0.95 {
        dsp::HIGH_CUT
    } else if t > 0.85 {
        dsp::HIGH_SHELF
    } else {
        dsp::BELL
    };
    let gain = if dsp::is_cut(shape) { 0.0 } else { da.db(at.y).clamp(da.lo, da.hi) };
    for (key, v) in [
        (dsp::K_SHAPE, shape as f32),
        (dsp::K_SLOPE, 1.0),
        (dsp::K_PLACE, 0.0),
        (dsp::K_FREQ, fa.f(at.x).clamp(10.0, 30_000.0)),
        (dsp::K_Q, if shape == dsp::BELL { 1.0 } else { std::f32::consts::FRAC_1_SQRT_2 }),
        (dsp::K_GAIN, gain),
        (dsp::K_DYN, 0.0),
        (dsp::K_SC, 0.0),
        (dsp::K_RANGE, -6.0),
        (dsp::K_THR, -20.0),
        (dsp::K_BYP, 0.0),
        (dsp::K_USED, 1.0),
    ] {
        cx.set(k(b, key), v);
    }
    st.selected = Some(b);
}

fn remove(cx: &mut Ctx, st: &mut State, b: usize) {
    if cx.idx("solo") == b + 1 {
        cx.set("solo", 0.0);
    }
    cx.set(k(b, dsp::K_USED), 0.0);
    if st.selected == Some(b) {
        st.selected = None;
    }
}

fn controls(ui: &mut Ui, cx: &mut Ctx, st: &mut State, strip: Rect) {
    let p = &ui.painter().clone();
    p.rect_filled(strip, 3.0, PLATE_HI);
    let cell = vec2(74.0, 92.0);
    let row = strip.min.y + 8.0;
    let mut x = strip.min.x + 8.0;
    if let Some(b) = st.selected.filter(|&b| used(cx, b)) {
        let col = band_color(b);
        let s = dsp::band_from(|id| cx.get(id), b);
        p.text(pos2(x, row + 4.0), Align2::LEFT_TOP, format!("BAND {}", b + 1), bold(12.0), col);
        let menus = [(dsp::K_SHAPE, dsp::SHAPES), (dsp::K_SLOPE, dsp::SLOPES), (dsp::K_PLACE, dsp::PLACES)];
        for (i, (key, labels)) in menus.into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(x, row + 24.0 + i as f32 * 24.0), vec2(104.0, 20.0));
            let enabled = key != dsp::K_SLOPE || dsp::has_slope(s.shape);
            if enabled && let Some(v) = choice(ui, r, (cx.id, b, key), labels, cx.idx(k(b, key))) {
                cx.set(k(b, key), v as f32);
            }
        }
        x += 116.0;
        knob(ui, cx, Rect::from_min_size(pos2(x, row), cell), k(b, dsp::K_FREQ), "Freq", col);
        x += cell.x;
        if dsp::has_gain(s.shape) {
            knob(ui, cx, Rect::from_min_size(pos2(x, row), cell), k(b, dsp::K_GAIN), "Gain", col);
        }
        x += cell.x;
        knob(ui, cx, Rect::from_min_size(pos2(x, row), cell), k(b, dsp::K_Q), "Q", col);
        x += cell.x + 8.0;
        let dynamic = s.dynamic;
        if dsp::has_gain(s.shape) {
            if button(ui, Rect::from_min_size(pos2(x, row + 14.0), vec2(64.0, 20.0)), "DYNAMIC", dynamic, READOUT).clicked() {
                cx.set(k(b, dsp::K_DYN), if dynamic { 0.0 } else { 1.0 });
            }
            if button(ui, Rect::from_min_size(pos2(x, row + 40.0), vec2(64.0, 20.0)), "EXT KEY", s.sc, ACCENT)
                .on_hover_text(t!("The band's detector listens to the key input"))
                .clicked()
            {
                cx.set(k(b, dsp::K_SC), if s.sc { 0.0 } else { 1.0 });
            }
            x += 72.0;
            if dynamic {
                knob(ui, cx, Rect::from_min_size(pos2(x, row), cell), k(b, dsp::K_THR), "Threshold", READOUT);
                knob(ui, cx, Rect::from_min_size(pos2(x + cell.x, row), cell), k(b, dsp::K_RANGE), "Range", READOUT);
            }
            x += 2.0 * cell.x;
        }
        let solo = cx.idx("solo") == b + 1;
        let bx = x + 6.0;
        if button(ui, Rect::from_min_size(pos2(bx, row + 4.0), vec2(60.0, 20.0)), "SOLO", solo, Color32::from_rgb(222, 196, 52)).clicked() {
            cx.set("solo", if solo { 0.0 } else { (b + 1) as f32 });
        }
        let byp = cx.on(k(b, dsp::K_BYP));
        if button(ui, Rect::from_min_size(pos2(bx, row + 28.0), vec2(60.0, 20.0)), "BYPASS", byp, GR).clicked() {
            cx.set(k(b, dsp::K_BYP), if byp { 0.0 } else { 1.0 });
        }
        if button(ui, Rect::from_min_size(pos2(bx, row + 52.0), vec2(60.0, 20.0)), "REMOVE", false, GR).clicked() {
            remove(cx, st, b);
        }
    } else {
        p.text(pos2(x, row + 30.0), Align2::LEFT_CENTER, t!("Double-click the display to add a band."), regular(12.0), DIM);
        p.text(pos2(x, row + 50.0), Align2::LEFT_CENTER, t!("Drag a handle, wheel for Q, right-click to remove."), regular(11.0), DIM);
    }
    let gx = strip.max.x - 8.0 - cell.x - 128.0;
    knob(ui, cx, Rect::from_min_size(pos2(gx + 128.0, row), cell), "out", "Output", ACCENT);
    let lines: [(&str, &[&str], usize); 4] = [
        ("phase", dsp::PHASES, cx.idx("phase")),
        ("res", dsp::RESOLUTIONS, cx.idx("res")),
        ("@an", &["Pre + Post", "Pre", "Post", "Analyzer off"], st.analyzer),
        ("@range", &["±3 dB", "±6 dB", "±12 dB", "±30 dB"], st.range),
    ];
    let mut i = 0;
    for (key, labels, cur) in lines {
        if key == "res" && cx.idx("phase") == 0 {
            continue;
        }
        let r = Rect::from_min_size(pos2(gx, row + i as f32 * 23.0), vec2(120.0, 20.0));
        i += 1;
        if let Some(v) = choice(ui, r, (cx.id, key), labels, cur) {
            match key {
                "@an" => st.analyzer = v,
                "@range" => st.range = v,
                k => cx.set(k, v as f32),
            }
        }
    }
}
