use super::*;
use openpt_dsp::plugins::open_r::{self as dsp, ALGORITHMS, NONLINEAR, SIZES};

pub const W: f32 = 760.0;
const H: f32 = 470.0;
const FLOOR: f32 = -60.0;

pub fn show(ui: &mut Ui, cx: &mut Ctx) {
    let (body, _) = plate(ui, vec2(W, H), "R");
    let algo = cx.idx("algorithm").min(ALGORITHMS.len() - 1);
    let size = cx.idx("size").min(SIZES.len() - 1);
    let top = body.min.y + 10.0;
    let n = ALGORITHMS.len() as f32;
    let bw = (W - 24.0 - 3.0 * 62.0 - 14.0 - (n - 1.0) * 4.0) / n;
    let mut x = body.min.x + 12.0;
    for (k, name) in ALGORITHMS.iter().enumerate() {
        if button(ui, Rect::from_min_size(pos2(x, top), vec2(bw, 24.0)), name, k == algo, ACCENT).clicked() && k != algo {
            cx.set("algorithm", k as f32);
            cx.set("decay", dsp::default_decay(k, size));
        }
        x += bw + 4.0;
    }
    x += 14.0;
    for (k, name) in SIZES.iter().enumerate() {
        if button(ui, Rect::from_min_size(pos2(x, top), vec2(58.0, 24.0)), name, k == size, READOUT).clicked() && k != size {
            cx.set("size", k as f32);
            cx.set("decay", dsp::default_decay(algo, k));
        }
        x += 62.0;
    }
    let g = Rect::from_min_size(pos2(body.min.x + 12.0, top + 34.0), vec2(W - 24.0, 220.0));
    response(ui, cx, g, algo, size);
    let strip = Rect::from_min_max(pos2(body.min.x + 12.0, g.max.y + 10.0), body.max - vec2(12.0, 8.0));
    controls(ui, cx, strip, algo);
}

fn response(ui: &mut Ui, cx: &Ctx, g: Rect, algo: usize, size: usize) {
    let p = ui.painter_at(g.expand(2.0));
    display(&p, g);
    let a = dsp::algo(algo);
    let scale = dsp::size_scale(size);
    let nonlinear = algo == NONLINEAR;
    let pre = cx.get("pre_delay") * 0.001;
    let decay = if nonlinear { dsp::gate_secs(cx.get("decay")) } else { cx.get("decay") };
    let span = (pre + decay * if nonlinear { 1.6 } else { 1.15 } + 0.05).max(0.2);
    let plot = g.shrink2(vec2(0.0, 6.0)).with_max_y(g.max.y - 16.0);
    let da = DbAxis { r: plot, lo: FLOOR, hi: 0.0 };
    da.grid(&p, 12.0, true);
    let tx = |t: f32| plot.min.x + plot.width() * (t / span).clamp(0.0, 1.0);
    let step = [0.01f32, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0].into_iter().find(|s| span / s <= 10.0).unwrap_or(10.0);
    let mut t = step;
    while t < span {
        p.vline(tx(t), plot.y_range(), Stroke::new(1.0, GRID));
        let label = if step < 1.0 { format!("{:.0} ms", t * 1000.0) } else { format!("{t:.0} s") };
        p.text(pos2(tx(t), g.max.y - 3.0), Align2::CENTER_BOTTOM, label, regular(9.0), DIM);
        t += step;
    }
    p.vline(tx(0.0) + 1.0, plot.y_range(), Stroke::new(2.0, DIM));
    if pre > 0.0 {
        p.add(egui::Shape::dashed_line(&[pos2(tx(0.0), da.y(-6.0)), pos2(tx(pre), da.y(-6.0))], Stroke::new(1.0, DIM), 4.0, 3.0));
        p.text(pos2(tx(pre * 0.5), da.y(-6.0) - 3.0), Align2::CENTER_BOTTOM, "pre-delay", regular(9.0), DIM);
    }
    let tail_col = ACCENT;
    let fill = Color32::from_rgba_unmultiplied(74, 47, 143, 110);
    let (early, late) = dsp::balance_gains(cx.get("balance"));
    let start = pre + a.tail_delay_ms * scale * 0.001;
    let level = 20.0 * (a.tail_level * late).max(1.0e-3).log10() - 6.0;
    let n = 120;
    let mut pts = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = start + (span - start) * i as f32 / n as f32;
        let age = t - start;
        let db = if nonlinear {
            let u = age / decay.max(1.0e-3);
            if u <= 1.0 { level + 20.0 * dsp::gate_shape(u).max(1.0e-3).log10() } else { FLOOR }
        } else {
            let build = (age / (0.012 * scale + 0.004)).min(1.0);
            level + FLOOR * age / decay.max(0.05) + 20.0 * build.max(1.0e-3).log10()
        };
        pts.push(pos2(tx(t), da.y(db.max(FLOOR))));
    }
    let mut mesh = egui::Mesh::default();
    for (i, q) in pts.iter().enumerate() {
        mesh.colored_vertex(*q, fill);
        mesh.colored_vertex(pos2(q.x, plot.max.y), fill.gamma_multiply(0.25));
        if i > 0 {
            let k = (2 * i) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    if late > 0.0 {
        p.add(mesh);
        p.add(egui::Shape::line(pts, Stroke::new(1.5, tail_col)));
    }
    for &(ms, gain, _) in a.er.iter().filter(|_| early > 0.0) {
        let t = pre + ms * scale * 0.001;
        let db = 20.0 * (gain * a.er_level * early).max(1.0e-3).log10();
        p.line_segment([pos2(tx(t), plot.max.y), pos2(tx(t), da.y(db.max(FLOOR)))], Stroke::new(1.5, READOUT));
    }
    let what = if nonlinear { format!("gate {:.0} ms", decay * 1000.0) } else { format!("RT60 {decay:.2} s") };
    p.text(
        pos2(plot.min.x + 8.0, plot.min.y + 4.0),
        Align2::LEFT_TOP,
        format!("{} · {} · {what}", ALGORITHMS[algo], SIZES[size]),
        regular(11.0),
        TEXT,
    );
    if !a.er.is_empty() && early > 0.0 {
        p.text(pos2(plot.min.x + 8.0, plot.min.y + 20.0), Align2::LEFT_TOP, "early reflections", regular(9.0), READOUT);
    }
}

fn controls(ui: &mut Ui, cx: &mut Ctx, strip: Rect, algo: usize) {
    let p = &ui.painter().clone();
    p.rect_filled(strip, 3.0, PLATE_HI);
    let cell = vec2(84.0, 96.0);
    let row = strip.min.y + 10.0;
    let mut x = strip.min.x + 10.0;
    p.text(pos2(x, row - 4.0), Align2::LEFT_TOP, "LEVEL", bold(10.0), ACCENT);
    for (id, label) in [("input", "Input"), ("mix", "Mix")] {
        knob(ui, cx, Rect::from_min_size(pos2(x, row + 16.0), cell), id, label, READOUT);
        x += cell.x;
    }
    x += 12.0;
    p.vline(x - 8.0, strip.min.y + 6.0..=strip.max.y - 6.0, Stroke::new(1.0, EDGE));
    p.text(pos2(x, row - 4.0), Align2::LEFT_TOP, "REVERB", bold(10.0), ACCENT);
    if let Some(prm) = cx.param("decay") {
        let v = cx.get("decay");
        let nonlinear = algo == NONLINEAR;
        let label = if nonlinear { "Gate" } else { "Decay" };
        knob_with(ui, cx, Rect::from_min_size(pos2(x, row + 16.0), cell), prm, v, label, ACCENT, &|v| {
            if nonlinear { format!("{:.0} ms", dsp::gate_secs(v) * 1000.0) } else { prm.format(v) }
        });
    }
    x += cell.x;
    for (id, label) in [("pre_delay", "Pre-Delay"), ("diffusion", "Diffusion")] {
        knob(ui, cx, Rect::from_min_size(pos2(x, row + 16.0), cell), id, label, ACCENT);
        x += cell.x;
    }
    if let Some(prm) = cx.param("balance") {
        let v = cx.get("balance");
        knob_with(ui, cx, Rect::from_min_size(pos2(x, row + 16.0), cell), prm, v, "Early/Late", ACCENT, &balance_text);
    }
    x += cell.x;
    x += 12.0;
    p.vline(x - 8.0, strip.min.y + 6.0..=strip.max.y - 6.0, Stroke::new(1.0, EDGE));
    p.text(pos2(x, row - 4.0), Align2::LEFT_TOP, "TONE", bold(10.0), ACCENT);
    for (id, label) in [("hf_cut", "HF Cut"), ("lp_filter", "LP Filter")] {
        if let Some(prm) = cx.param(id) {
            let v = cx.get(id);
            knob_with(ui, cx, Rect::from_min_size(pos2(x, row + 16.0), cell), prm, v, label, Color32::from_rgb(90, 200, 250), &|v| {
                if v >= 19_500.0 { "Off".to_string() } else { prm.format(v) }
            });
        }
        x += cell.x;
    }
}

fn balance_text(v: f32) -> String {
    let (e, l) = dsp::balance_gains(v);
    match (e, l) {
        (_, 0.0) => "Early".to_string(),
        (0.0, _) => "Late".to_string(),
        (1.0, 1.0) => "Early + Late".to_string(),
        (1.0, l) => format!("Late {:.0}%", l * 100.0),
        (e, _) => format!("Early {:.0}%", e * 100.0),
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, pos2};
    use std::collections::BTreeMap;

    #[test]
    fn picking_an_algorithm_sets_its_decay_and_every_one_draws() {
        let info = openpt_dsp::plugin_info("open_r").expect("OpenR is registered");
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let mut params = BTreeMap::new();
        for algo in 0..super::ALGORITHMS.len() {
            params.insert("algorithm".to_string(), algo as f32);
            let mut edits = Vec::new();
            let mut full = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(super::super::panel(ui, info, &params, None, egui::Id::new("t"), &mut |id, v| edits.push((id.to_string(), v))));
            });
            full.textures_delta.clear();
            assert!(edits.is_empty());
        }
        let at = pos2(12.0 + 60.0 + 30.0, 31.0 + 10.0 + 12.0);
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let mut edits = Vec::new();
        params.insert("algorithm".to_string(), 0.0);
        for events in [vec![Event::PointerMoved(at), press(true)], vec![press(false)]] {
            let mut full = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                super::super::panel(ui, info, &params, None, egui::Id::new("t"), &mut |id, v| edits.push((id.to_string(), v)));
            });
            full.textures_delta.clear();
        }
        assert_eq!(edits, vec![("algorithm".to_string(), 1.0), ("decay".to_string(), openpt_dsp::plugins::open_r::default_decay(1, 1))]);
    }
}
