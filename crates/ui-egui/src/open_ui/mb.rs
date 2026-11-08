use super::*;
use openpt_dsp::plugins::open_mb::{self as dsp, BANDS};

pub const W: f32 = 980.0;
const H: f32 = 620.0;
const RANGES: [f32; 4] = [6.0, 12.0, 24.0, 30.0];

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Dot,
    EdgeLo,
    EdgeHi,
    RangeLine,
    Body,
}

#[derive(Default)]
struct State {
    selected: Option<usize>,
    drag: Option<(usize, Hit, Pos2, f32, f32, f32)>,
    range: usize,
    analyzer: usize,
    smooth: Smoothed,
    init: bool,
}

fn k(b: usize, key: usize) -> &'static str {
    dsp::band_param(b, key)
}

fn used(cx: &Ctx, b: usize) -> bool {
    cx.on(k(b, dsp::K_USED))
}

fn centre(cx: &Ctx, b: usize) -> f32 {
    let lo = cx.get(k(b, dsp::K_LO)).max(20.0);
    let hi = cx.get(k(b, dsp::K_HI)).min(20_000.0);
    (lo * hi).sqrt()
}

pub fn show(ui: &mut Ui, cx: &mut Ctx) {
    let (body, _) = plate(ui, vec2(W, H), "MB");
    with_state::<State, _>(cx.id, |st| {
        if !st.init {
            st.init = true;
            st.range = 2;
        }
        let g = Rect::from_min_size(body.min + vec2(12.0, 10.0), vec2(W - 24.0, 360.0));
        graph(ui, cx, st, g);
        let strip = Rect::from_min_max(pos2(body.min.x + 12.0, g.max.y + 10.0), body.max - vec2(12.0, 8.0));
        controls(ui, cx, st, strip);
    });
}

fn neighbours(cx: &Ctx, b: usize) -> (f32, f32) {
    let (mut lo, mut hi) = (dsp::MIN_F, dsp::MAX_F);
    let c = centre(cx, b);
    for o in (0..BANDS).filter(|&o| o != b && used(cx, o)) {
        if centre(cx, o) < c {
            lo = lo.max(cx.get(k(o, dsp::K_HI)));
        } else {
            hi = hi.min(cx.get(k(o, dsp::K_LO)));
        }
    }
    (lo, hi)
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
            Color32::from_rgba_unmultiplied(74, 47, 143, 100),
            Color32::from_rgba_unmultiplied(184, 164, 255, 110),
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
    let live: Vec<f32> = cx.scope.as_ref().map(|s| s.read(|d| d.bands[..BANDS].to_vec())).unwrap_or_else(|| vec![0.0; BANDS]);
    let cols = (g.width() / 3.0) as usize;
    let xs: Vec<f32> = (0..=cols).map(|i| g.min.x + g.width() * i as f32 / cols as f32).collect();
    let mut total = vec![1.0f64; xs.len()];
    let dot_y = |cx: &Ctx, b: usize| da.y(cx.get(k(b, dsp::K_LEVEL))).clamp(g.min.y + 8.0, g.max.y - 8.0);
    let range_y = |cx: &Ctx, b: usize| da.y(cx.get(k(b, dsp::K_LEVEL)) + cx.get(k(b, dsp::K_RANGE))).clamp(g.min.y + 2.0, g.max.y - 2.0);
    for b in (0..BANDS).filter(|&b| used(cx, b)) {
        let s = dsp::band_from(|id| cx.get(id), b);
        let sh = dsp::shape_of(&s, fs);
        let col = band_color(b);
        let sel = st.selected == Some(b);
        let (xl, xh) = (fa.x(s.lo).max(g.min.x), fa.x(s.hi).min(g.max.x));
        p.rect_filled(
            Rect::from_x_y_ranges(xl..=xh, g.y_range()),
            0.0,
            Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), if sel { 22 } else { 10 }),
        );
        let gdb = if s.active { s.level + live[b] } else { 0.0 };
        let gl = 10f64.powf(gdb as f64 / 20.0);
        let ys: Vec<f32> = xs
            .iter()
            .map(|&x| {
                let m = dsp::band_mag(&sh, fa.f(x) as f64);
                (20.0 * (1.0 + (gl - 1.0) * m).max(1.0e-6).log10()) as f32
            })
            .collect();
        for (t, &x) in total.iter_mut().zip(&xs) {
            if s.active {
                *t *= 1.0 + (gl - 1.0) * dsp::band_mag(&sh, fa.f(x) as f64);
            }
        }
        let mut mesh = egui::Mesh::default();
        let fill = Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), if s.active { 70 } else { 15 });
        let zero = da.y(0.0);
        for (i, (&x, &y)) in xs.iter().zip(&ys).enumerate() {
            mesh.colored_vertex(pos2(x, da.y(y).clamp(g.min.y, g.max.y)), fill);
            mesh.colored_vertex(pos2(x, zero), fill);
            if i > 0 {
                let kk = (2 * i) as u32;
                mesh.add_triangle(kk - 2, kk - 1, kk);
                mesh.add_triangle(kk - 1, kk, kk + 1);
            }
        }
        p.add(mesh);
        if sel {
            for (x, open) in [(xl, s.lo <= dsp::OPEN_LO), (xh, s.hi >= dsp::OPEN_HI)] {
                if !open {
                    p.vline(x, g.y_range(), Stroke::new(1.5, col));
                }
            }
            let ry = range_y(cx, b);
            p.add(egui::Shape::dashed_line(&[pos2(xl, ry), pos2(xh, ry)], Stroke::new(1.0, col), 5.0, 4.0));
        }
        let c = pos2(fa.x(centre(cx, b)), dot_y(cx, b));
        p.circle(c, 8.0, if s.active { col } else { DISPLAY }, Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { Color32::WHITE } else { col }));
        p.text(c, Align2::CENTER_CENTER, format!("{}", b + 1), bold(9.0), if s.active { Color32::from_rgb(16, 14, 22) } else { col });
        if live[b].abs() > 0.05 {
            p.text(pos2(c.x, c.y + 18.0), Align2::CENTER_CENTER, format!("{:+.1}", live[b]), regular(9.5), READOUT);
        }
    }
    let pts: Vec<Pos2> = xs.iter().zip(&total).map(|(&x, &t)| pos2(x, da.y((20.0 * t.max(1.0e-6).log10()) as f32).clamp(g.min.y, g.max.y))).collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.0, Color32::from_rgb(236, 232, 255))));
    let hit_test = |cx: &Ctx, at: Pos2, sel: Option<usize>| -> Option<(usize, Hit)> {
        for b in (0..BANDS).filter(|&b| used(cx, b)) {
            if pos2(fa.x(centre(cx, b)), dot_y(cx, b)).distance(at) < 16.0 {
                return Some((b, Hit::Dot));
            }
        }
        if let Some(b) = sel.filter(|&b| used(cx, b)) {
            let (xl, xh) = (fa.x(cx.get(k(b, dsp::K_LO))), fa.x(cx.get(k(b, dsp::K_HI))));
            if at.x > xl && at.x < xh && (at.y - range_y(cx, b)).abs() < 8.0 {
                return Some((b, Hit::RangeLine));
            }
            if cx.get(k(b, dsp::K_LO)) > dsp::OPEN_LO && (at.x - xl).abs() < 8.0 {
                return Some((b, Hit::EdgeLo));
            }
            if cx.get(k(b, dsp::K_HI)) < dsp::OPEN_HI && (at.x - xh).abs() < 8.0 {
                return Some((b, Hit::EdgeHi));
            }
        }
        (0..BANDS)
            .filter(|&b| used(cx, b))
            .find(|&b| at.x >= fa.x(cx.get(k(b, dsp::K_LO))) && at.x <= fa.x(cx.get(k(b, dsp::K_HI))))
            .map(|b| (b, Hit::Body))
    };
    let hover = resp.hover_pos().and_then(|at| hit_test(cx, at, st.selected));
    match hover.map(|h| h.1) {
        Some(Hit::EdgeLo | Hit::EdgeHi) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
        Some(Hit::RangeLine) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical),
        Some(Hit::Dot) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
        _ => {}
    }
    if resp.drag_started()
        && let Some(at) = ui.input(|i| i.pointer.press_origin()).or_else(|| resp.interact_pointer_pos())
    {
        st.drag = None;
        if let Some((b, h)) = hit_test(cx, at, st.selected) {
            st.selected = Some(b);
            if h != Hit::Body {
                let v = if h == Hit::RangeLine { cx.get(k(b, dsp::K_RANGE)) } else { cx.get(k(b, dsp::K_LEVEL)) };
                st.drag = Some((b, h, at, cx.get(k(b, dsp::K_LO)), cx.get(k(b, dsp::K_HI)), v));
            }
        }
    }
    if resp.dragged()
        && let (Some((b, h, start, lo0, hi0, v0)), Some(at)) = (st.drag, resp.interact_pointer_pos())
    {
        let fine = if ui.input(|i| i.modifiers.shift) { 0.15 } else { 1.0 };
        let alt = ui.input(|i| i.modifiers.alt);
        let (mut dx, mut dy) = ((at.x - start.x) * fine, (at.y - start.y) * fine);
        let (nlo, nhi) = neighbours(cx, b);
        let min_ratio = 2f32.powf(1.0 / 6.0);
        match h {
            Hit::Dot => {
                if !alt {
                    if dx.abs() > dy.abs() {
                        dy = 0.0;
                    } else {
                        dx = 0.0;
                    }
                }
                let w = fa.x(hi0) - fa.x(lo0);
                let xl = (fa.x(lo0) + dx).clamp(fa.x(nlo), fa.x(nlo).max(fa.x(nhi) - w));
                let lo = fa.f(xl).max(nlo);
                let hi = fa.f(xl + w).min(nhi);
                cx.set(k(b, dsp::K_LO), lo);
                cx.set(k(b, dsp::K_HI), hi.max(lo * min_ratio));
                let ddb = da.db(start.y + dy) - da.db(start.y);
                cx.set(k(b, dsp::K_LEVEL), (v0 + ddb).clamp(-30.0, 30.0));
            }
            Hit::EdgeLo => {
                let f = fa.f(fa.x(lo0) + dx);
                cx.set(k(b, dsp::K_LO), f.clamp(nlo, cx.get(k(b, dsp::K_HI)) / min_ratio));
            }
            Hit::EdgeHi => {
                let f = fa.f(fa.x(hi0) + dx);
                cx.set(k(b, dsp::K_HI), f.clamp(cx.get(k(b, dsp::K_LO)) * min_ratio, nhi));
            }
            Hit::RangeLine => {
                let ddb = da.db(start.y + dy) - da.db(start.y);
                cx.set(k(b, dsp::K_RANGE), (v0 + ddb).clamp(-30.0, 30.0));
            }
            Hit::Body => {}
        }
    }
    if resp.drag_stopped() {
        st.drag = None;
    }
    if resp.clicked()
        && let Some(at) = resp.interact_pointer_pos()
    {
        st.selected = hit_test(cx, at, st.selected).map(|h| h.0);
    }
    if resp.secondary_clicked()
        && let Some((b, Hit::Dot)) = resp.interact_pointer_pos().and_then(|at| hit_test(cx, at, st.selected))
    {
        cx.set(k(b, dsp::K_SOLO), 0.0);
        cx.set(k(b, dsp::K_USED), 0.0);
        if st.selected == Some(b) {
            st.selected = None;
        }
    }
    if resp.double_clicked()
        && let Some(at) = resp.interact_pointer_pos()
        && hit_test(cx, at, None).is_none()
    {
        add(cx, st, fa.f(at.x));
    }
    if let Some((b, _)) = hover.or(st.drag.map(|d| (d.0, d.1))) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        if wheel != 0.0 {
            let (nlo, nhi) = neighbours(cx, b);
            let c = centre(cx, b);
            let half = (cx.get(k(b, dsp::K_HI)) / cx.get(k(b, dsp::K_LO))).sqrt();
            let nh = (half * 2f32.powf(wheel * 0.002)).clamp(2f32.powf(1.0 / 12.0), 64.0);
            cx.set(k(b, dsp::K_LO), (c / nh).max(nlo));
            cx.set(k(b, dsp::K_HI), (c * nh).min(nhi));
        }
    }
}

fn add(cx: &mut Ctx, st: &mut State, f: f32) {
    let Some(b) = (0..BANDS).find(|&b| !used(cx, b)) else { return };
    let (mut lo, mut hi) = (dsp::MIN_F, dsp::MAX_F);
    for o in (0..BANDS).filter(|&o| used(cx, o)) {
        let (ol, oh) = (cx.get(k(o, dsp::K_LO)), cx.get(k(o, dsp::K_HI)));
        if ol <= f && f <= oh {
            return;
        }
        if oh <= f {
            lo = lo.max(oh);
        }
        if ol >= f {
            hi = hi.min(ol);
        }
    }
    let (bl, bh) = (lo.max(f * 0.5), hi.min(f * 2.0));
    if bh / bl < 2f32.powf(1.0 / 3.0) {
        return;
    }
    for (key, v) in [
        (dsp::K_LO, bl),
        (dsp::K_HI, bh),
        (dsp::K_SLO, 24.0),
        (dsp::K_SHI, 24.0),
        (dsp::K_MODE, 0.0),
        (dsp::K_THR, -18.0),
        (dsp::K_RANGE, -6.0),
        (dsp::K_RATIO, 4.0),
        (dsp::K_KNEE, 24.0),
        (dsp::K_ATK, 20.0),
        (dsp::K_REL, 20.0),
        (dsp::K_LA, 1.0),
        (dsp::K_LEVEL, 0.0),
        (dsp::K_LATK, 16.03),
        (dsp::K_LREL, 25.0),
        (dsp::K_PLACE, 0.0),
        (dsp::K_SC, 0.0),
        (dsp::K_SOLO, 0.0),
        (dsp::K_BYP, 0.0),
        (dsp::K_USED, 1.0),
    ] {
        cx.set(k(b, key), v);
    }
    st.selected = Some(b);
}

fn controls(ui: &mut Ui, cx: &mut Ctx, st: &mut State, strip: Rect) {
    let p = &ui.painter().clone();
    p.rect_filled(strip, 3.0, PLATE_HI);
    let cell = vec2(66.0, 84.0);
    let row = strip.min.y + 8.0;
    let row2 = row + cell.y + 4.0;
    let range_law = cx.idx("law") == dsp::LAW_RANGE;
    let mut x = strip.min.x + 8.0;
    if let Some(b) = st.selected.filter(|&b| used(cx, b)) {
        let col = band_color(b);
        p.text(pos2(x, row + 4.0), Align2::LEFT_TOP, format!("BAND {}", b + 1), bold(12.0), col);
        for (i, (key, labels)) in [(dsp::K_MODE, dsp::MODES), (dsp::K_PLACE, dsp::PLACES)].into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(x, row + 24.0 + i as f32 * 24.0), vec2(100.0, 20.0));
            if let Some(v) = choice(ui, r, (cx.id, b, key), labels, cx.idx(k(b, key))) {
                cx.set(k(b, key), v as f32);
            }
        }
        let flags: [(usize, &str, Color32); 3] =
            [(dsp::K_SOLO, "SOLO", Color32::from_rgb(222, 196, 52)), (dsp::K_BYP, "BYPASS", GR), (dsp::K_SC, "EXT KEY", ACCENT)];
        for (i, (key, label, lit)) in flags.into_iter().enumerate() {
            let on = cx.on(k(b, key));
            if button(ui, Rect::from_min_size(pos2(x, row2 + 4.0 + i as f32 * 24.0), vec2(100.0, 20.0)), label, on, lit).clicked() {
                cx.set(k(b, key), if on { 0.0 } else { 1.0 });
            }
        }
        x += 112.0;
        let top: Vec<(usize, &str)> = if range_law {
            vec![(dsp::K_THR, "Threshold"), (dsp::K_RANGE, "Range"), (dsp::K_LATK, "Attack"), (dsp::K_LREL, "Release"), (dsp::K_LEVEL, "Level")]
        } else {
            vec![(dsp::K_THR, "Threshold"), (dsp::K_RATIO, "Ratio"), (dsp::K_RANGE, "Range"), (dsp::K_KNEE, "Knee"), (dsp::K_LEVEL, "Level")]
        };
        for (i, (key, label)) in top.into_iter().enumerate() {
            knob(ui, cx, Rect::from_min_size(pos2(x + i as f32 * cell.x, row), cell), k(b, key), label, col);
        }
        let bottom: Vec<(usize, &str)> = if range_law {
            vec![(dsp::K_SLO, "Low Slope"), (dsp::K_SHI, "High Slope")]
        } else {
            vec![(dsp::K_ATK, "Attack"), (dsp::K_REL, "Release"), (dsp::K_LA, "Lookahead"), (dsp::K_SLO, "Low Slope"), (dsp::K_SHI, "High Slope")]
        };
        for (i, (key, label)) in bottom.into_iter().enumerate() {
            knob(ui, cx, Rect::from_min_size(pos2(x + i as f32 * cell.x, row2), cell), k(b, key), label, col);
        }
    } else {
        p.text(pos2(x, row + 30.0), Align2::LEFT_CENTER, t!("Double-click the display to add a band."), regular(12.0), DIM);
        p.text(
            pos2(x, row + 50.0),
            Align2::LEFT_CENTER,
            t!("Drag the dot (Alt: freely), the edges and the dashed range line; wheel to widen."),
            regular(11.0),
            DIM,
        );
    }
    let gx = strip.max.x - 8.0 - 2.0 * cell.x - 128.0;
    knob(ui, cx, Rect::from_min_size(pos2(gx + 128.0, row), cell), "mix", "Mix", ACCENT);
    knob(ui, cx, Rect::from_min_size(pos2(gx + 128.0 + cell.x, row), cell), "out", "Output", ACCENT);
    if range_law {
        knob(ui, cx, Rect::from_min_size(pos2(gx + 128.0, row2), cell), "lknee", "Range Knee", ACCENT);
    }
    let lines: [(&str, &[&str], usize); 7] = [
        ("law", dsp::LAWS, cx.idx("law")),
        ("relmode", &["ARC", "Manual"], cx.idx("relmode")),
        ("behavior", &["Electro", "Opto"], cx.idx("behavior")),
        ("phase", dsp::PHASES, cx.idx("phase")),
        ("lookahead", &["Lookahead Off", "Lookahead On"], cx.idx("lookahead")),
        ("@an", &["Pre + Post", "Pre", "Post", "Analyzer off"], st.analyzer),
        ("@range", &["±6 dB", "±12 dB", "±24 dB", "±30 dB"], st.range),
    ];
    let mut i = 0;
    for (key, labels, cur) in lines {
        if !range_law && (key == "relmode" || key == "behavior") {
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
