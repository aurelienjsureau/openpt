use super::*;
use openpt_dsp::plugins::open_echo::{self as dsp, DUAL, MODES, NOTES, PING_PONG, STYLES};

pub const W: f32 = 920.0;
const H: f32 = 470.0;
const FLOOR: f32 = -48.0;
const TAPE: Color32 = Color32::from_rgb(236, 206, 84);

#[derive(Default)]
struct EchoUi {
    advanced: bool,
}

pub fn show(ui: &mut Ui, cx: &mut Ctx) {
    let (body, head) = plate(ui, vec2(W, H), "Echo");
    let adv = with_state::<EchoUi, _>(cx.id, |st| {
        let b = Rect::from_min_size(pos2(head.max.x - 170.0, head.center().y - 10.0), vec2(90.0, 20.0));
        if button(ui, b, "ADVANCED", st.advanced, ACCENT).clicked() {
            st.advanced = !st.advanced;
        }
        st.advanced
    });
    if adv { advanced(ui, cx, body) } else { simple(ui, cx, body) }
}

fn simple(ui: &mut Ui, cx: &mut Ctx, body: Rect) {
    let style = cx.idx("style").min(STYLES.len() - 1);
    let mode = cx.idx("mode").min(MODES.len() - 1);
    let top = body.min.y + 10.0;
    let p = &ui.painter().clone();
    let mut x = body.min.x + 12.0;
    p.text(pos2(x, top + 12.0), Align2::LEFT_CENTER, "STYLE", bold(10.0), ACCENT);
    if let Some(k) = choice(ui, Rect::from_min_size(pos2(x + 44.0, top), vec2(170.0, 24.0)), (cx.id, "style"), STYLES, style) {
        cx.set("style", k as f32);
    }
    x += 240.0;
    p.text(pos2(x, top + 12.0), Align2::LEFT_CENTER, "MODE", bold(10.0), ACCENT);
    if let Some(k) = choice(ui, Rect::from_min_size(pos2(x + 40.0, top), vec2(130.0, 24.0)), (cx.id, "mode"), MODES, mode) {
        cx.set("mode", k as f32);
    }
    x += 200.0;
    let sync = cx.on("sync");
    if button(ui, Rect::from_min_size(pos2(x, top), vec2(70.0, 24.0)), "SYNC", sync, READOUT).clicked() {
        cx.set("sync", if sync { 0.0 } else { 1.0 });
    }
    let g = Rect::from_min_size(pos2(body.min.x + 12.0, top + 34.0), vec2(W - 24.0, 200.0));
    echoes(ui, cx, g, mode);
    let strip = Rect::from_min_max(pos2(body.min.x + 12.0, g.max.y + 10.0), body.max - vec2(12.0, 8.0));
    p.rect_filled(strip, 3.0, PLATE_HI);
    let cell = vec2(150.0, strip.height() - 12.0);
    let n = if mode == DUAL { 5.0 } else { 4.0 };
    let mut x = strip.center().x - n * cell.x * 0.5;
    let y = strip.min.y + 6.0;
    let (tl, tr) = times(cx);
    let ids: &[(&str, &str, &str, f32)] =
        if mode == DUAL { &[("time", "note", "Time L", tl), ("time_r", "note_r", "Time R", tr)] } else { &[("time", "note", "Time", tl)] };
    for &(t_id, n_id, name, ms) in ids {
        time_knob(ui, cx, Rect::from_min_size(pos2(x, y), cell), t_id, n_id, name, ms);
        x += cell.x;
    }
    for (id, label, color) in [("feedback", "Feedback", ACCENT), ("high_cut", "High Cut", TAPE), ("mix", "Mix", READOUT)] {
        knob(ui, cx, Rect::from_min_size(pos2(x, y), cell), id, label, color);
        x += cell.x;
    }
}

fn time_knob(ui: &mut Ui, cx: &mut Ctx, r: Rect, t_id: &str, n_id: &str, name: &str, ms: f32) {
    let sync = cx.on("sync");
    let id = if sync { n_id } else { t_id };
    let Some(prm) = cx.param(id) else { return };
    let v = cx.get(id);
    let text = |v: f32| if sync { NOTES[(v.round().max(0.0) as usize).min(NOTES.len() - 1)].to_string() } else { time_text(v) };
    knob_with(ui, cx, r, prm, v, name, ACCENT, &text);
    if sync {
        ui.painter().text(pos2(r.center().x, r.max.y - 2.0), Align2::CENTER_BOTTOM, time_text(ms), regular(9.0), DIM);
    }
}

fn advanced(ui: &mut Ui, cx: &mut Ctx, body: Rect) {
    let style = cx.idx("style").min(STYLES.len() - 1);
    let mode = cx.idx("mode").min(MODES.len() - 1);
    let top = body.min.y + 10.0;
    let mut x = body.min.x + 12.0;
    let sw = 92.0;
    for (k, name) in STYLES.iter().enumerate() {
        if button(ui, Rect::from_min_size(pos2(x, top), vec2(sw, 24.0)), name, k == style, TAPE).clicked() {
            cx.set("style", k as f32);
        }
        x += sw + 4.0;
    }
    x = body.max.x - 12.0 - 3.0 * 84.0 + 4.0;
    for (k, name) in MODES.iter().enumerate() {
        if button(ui, Rect::from_min_size(pos2(x, top), vec2(80.0, 24.0)), name, k == mode, ACCENT).clicked() {
            cx.set("mode", k as f32);
        }
        x += 84.0;
    }
    let g = Rect::from_min_size(pos2(body.min.x + 12.0, top + 34.0), vec2(W - 24.0, 200.0));
    echoes(ui, cx, g, mode);
    let strip = Rect::from_min_max(pos2(body.min.x + 12.0, g.max.y + 10.0), body.max - vec2(12.0, 8.0));
    controls(ui, cx, strip, mode);
}

fn times(cx: &Ctx) -> (f32, f32) {
    let tempo = cx.scope.as_ref().map_or(120.0, |s| f64::from(s.read(|d| d.readout)).max(0.0));
    let tempo = if tempo > 1.0 { tempo } else { 120.0 };
    dsp::times_ms(cx.on("sync"), cx.get("time"), note(cx, "note"), cx.get("time_r"), note(cx, "note_r"), cx.idx("mode"), tempo)
}

fn note(cx: &Ctx, id: &str) -> usize {
    (cx.get(id).round().max(0.0) as usize).min(NOTES.len() - 1)
}

fn time_text(ms: f32) -> String {
    if ms >= 1000.0 { format!("{:.2} s", ms / 1000.0) } else { format!("{ms:.0} ms") }
}

fn echoes(ui: &mut Ui, cx: &Ctx, g: Rect, mode: usize) {
    let p = ui.painter_at(g.expand(2.0));
    display(&p, g);
    let (tl, tr) = times(cx);
    let longest = if mode == PING_PONG { tl * (2.0 + cx.get("groove") / 100.0) } else { tl.max(tr) } * 0.001;
    let span = (longest * 8.0).clamp(0.3, 12.0);
    let plot = g.shrink2(vec2(10.0, 8.0)).with_max_y(g.max.y - 16.0);
    let mid = plot.center().y;
    let half = plot.height() * 0.5 - 4.0;
    let tx = |t: f32| plot.min.x + plot.width() * (t / span).clamp(0.0, 1.0);
    let step = [0.05f32, 0.1, 0.2, 0.25, 0.5, 1.0, 2.0].into_iter().find(|s| span / s <= 12.0).unwrap_or(5.0);
    let mut t = step;
    while t < span {
        p.vline(tx(t), plot.y_range(), Stroke::new(1.0, GRID));
        p.text(pos2(tx(t), g.max.y - 3.0), Align2::CENTER_BOTTOM, time_text(t * 1000.0), regular(9.0), DIM);
        t += step;
    }
    p.hline(plot.x_range(), mid, Stroke::new(1.0, GRID_STRONG));
    p.text(pos2(plot.min.x + 4.0, plot.min.y + 2.0), Align2::LEFT_TOP, "L", bold(10.0), DIM);
    p.text(pos2(plot.min.x + 4.0, plot.max.y - 2.0), Align2::LEFT_BOTTOM, "R", bold(10.0), DIM);
    p.line_segment([pos2(tx(0.0) + 1.0, mid - half), pos2(tx(0.0) + 1.0, mid + half)], Stroke::new(2.0, DIM));
    let list = dsp::echoes(mode, cx.get("feedback"), cx.get("groove"), (tl, tr), span);
    let mix = (cx.get("mix") / 100.0).max(0.05);
    for (at, level, side) in list {
        let db = 20.0 * (level * mix).max(1.0e-4).log10();
        let h = half * ((db - FLOOR) / -FLOOR).clamp(0.0, 1.0);
        let x = tx(at);
        let (a, b) = if side == 0 { (mid, mid - h) } else { (mid, mid + h) };
        p.line_segment([pos2(x, a), pos2(x, b)], Stroke::new(3.0, if side == 0 { ACCENT } else { READOUT }));
    }
    let what = match mode {
        DUAL => format!("{} · L {} · R {}", MODES[mode], label(cx, "note", tl), label(cx, "note_r", tr)),
        _ => format!("{} · {}", MODES[mode], label(cx, "note", tl)),
    };
    p.text(pos2(plot.max.x - 4.0, plot.min.y + 2.0), Align2::RIGHT_TOP, what, regular(11.0), TEXT);
}

fn label(cx: &Ctx, id: &str, ms: f32) -> String {
    if cx.on("sync") { format!("{} ({})", NOTES[note(cx, id)], time_text(ms)) } else { time_text(ms) }
}

fn controls(ui: &mut Ui, cx: &mut Ctx, strip: Rect, mode: usize) {
    let p = &ui.painter().clone();
    p.rect_filled(strip, 3.0, PLATE_HI);
    let cell = vec2(82.0, 96.0);
    let row = strip.min.y + 10.0;
    let mut x = strip.min.x + 10.0;
    p.text(pos2(x, row - 4.0), Align2::LEFT_TOP, "TIME", bold(10.0), ACCENT);
    let sync = cx.on("sync");
    if button(ui, Rect::from_min_size(pos2(x + 44.0, row - 6.0), vec2(48.0, 16.0)), "SYNC", sync, READOUT).clicked() {
        cx.set("sync", if sync { 0.0 } else { 1.0 });
    }
    let (tl, tr) = times(cx);
    for (k, (t_id, n_id)) in [("time", "note"), ("time_r", "note_r")].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, row + 16.0), cell);
        if k == 1 && mode != DUAL {
            if mode == PING_PONG {
                knob(ui, cx, r, "groove", "Groove", ACCENT);
            }
            x += cell.x;
            continue;
        }
        let name = if mode == DUAL { ["Time L", "Time R"][k] } else { "Time" };
        let ms = if k == 0 { tl } else { tr };
        let id = if sync { n_id } else { t_id };
        if let Some(prm) = cx.param(id) {
            let v = cx.get(id);
            let text = |v: f32| if sync { NOTES[(v.round().max(0.0) as usize).min(NOTES.len() - 1)].to_string() } else { time_text(v) };
            knob_with(ui, cx, r, prm, v, name, ACCENT, &text);
            if sync {
                p.text(pos2(r.center().x, r.max.y - 2.0), Align2::CENTER_BOTTOM, time_text(ms), regular(9.0), DIM);
            }
        }
        x += cell.x;
    }
    for (title, ids, color) in [
        ("FEEDBACK", &[("feedback", "Feedback"), ("diffusion", "Diffusion")][..], ACCENT),
        ("CHARACTER", &[("saturation", "Saturation"), ("wobble", "Wobble"), ("low_cut", "Low Cut"), ("high_cut", "High Cut")][..], TAPE),
        ("OUTPUT", &[("mix", "Mix"), ("output", "Output")][..], READOUT),
    ] {
        x += 12.0;
        p.vline(x - 8.0, strip.min.y + 6.0..=strip.max.y - 6.0, Stroke::new(1.0, EDGE));
        p.text(pos2(x, row - 4.0), Align2::LEFT_TOP, title, bold(10.0), ACCENT);
        for &(id, label) in ids {
            knob(ui, cx, Rect::from_min_size(pos2(x, row + 16.0), cell), id, label, color);
            x += cell.x;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    #[test]
    fn every_style_and_mode_draws() {
        let info = openpt_dsp::plugin_info("open_echo").expect("OpenEcho is registered");
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        for mode in 0..super::MODES.len() {
            for (style, sync) in [(0, true), (2, false), (5, true)] {
                let params = BTreeMap::from([
                    ("mode".to_string(), mode as f32),
                    ("style".to_string(), style as f32),
                    ("sync".to_string(), f32::from(u8::from(sync))),
                ]);
                let mut edits = Vec::new();
                let mut full = ctx.run_ui(egui::RawInput::default(), |ui| {
                    assert!(super::super::panel(ui, info, &params, None, egui::Id::new("t"), &mut |id, v| edits.push((id.to_string(), v))));
                });
                full.textures_delta.clear();
                assert!(edits.is_empty());
            }
        }
    }
}
