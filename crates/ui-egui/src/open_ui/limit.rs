use super::deess::{Meters, history, meter_bars};
use super::*;
use openpt_dsp::plugins::open_l::STYLES;

pub const W: f32 = 840.0;
const H: f32 = 400.0;
const LO: f32 = -48.0;
const GR_SPAN: f32 = 18.0;

pub fn show(ui: &mut Ui, cx: &mut Ctx) {
    let (body, _) = plate(ui, vec2(W, H), "L");
    with_state::<Meters, _>(cx.id, |st| {
        let hist = Rect::from_min_size(body.min + vec2(12.0, 10.0), vec2(600.0, 220.0));
        let meters = Rect::from_min_max(pos2(hist.max.x + 12.0, hist.min.y), pos2(body.max.x - 12.0, hist.max.y));
        history(ui, cx, st, hist, LO, Some("ceiling"), GR_SPAN);
        meter_bars(ui, cx, st, meters, LO, GR_SPAN);
        let strip = Rect::from_min_max(pos2(body.min.x + 12.0, hist.max.y + 10.0), body.max - vec2(12.0, 8.0));
        let p = &ui.painter().clone();
        p.rect_filled(strip, 3.0, PLATE_HI);
        let col = Rect::from_min_size(strip.min + vec2(8.0, 8.0), vec2(150.0, strip.height() - 16.0));
        p.text(col.min, Align2::LEFT_TOP, "STYLE", regular(9.0), DIM);
        let style = cx.idx("style");
        for (i, s) in STYLES.iter().enumerate() {
            let b = Rect::from_min_size(pos2(col.min.x, col.min.y + 14.0 + i as f32 * 26.0), vec2(col.width(), 22.0));
            if button(ui, b, s, i == style, ACCENT).clicked() {
                cx.set("style", i as f32);
            }
        }
        let big = Rect::from_min_size(pos2(col.max.x + 14.0, strip.min.y + 4.0), vec2(120.0, 120.0));
        knob(ui, cx, big, "gain", "Gain", READOUT);
        let cell = vec2(78.0, 100.0);
        let mut x = big.max.x + 8.0;
        for (id, label) in [("ceiling", "Output"), ("release", "Release"), ("lookahead", "Lookahead"), ("link", "Link")] {
            knob(ui, cx, Rect::from_min_size(pos2(x, strip.min.y + 12.0), cell), id, label, ACCENT);
            x += cell.x;
        }
        let on = cx.on("true_peak");
        let tb = Rect::from_min_size(pos2(x + 6.0, strip.min.y + 26.0), vec2(strip.max.x - x - 18.0, 26.0));
        if button(ui, tb, "TRUE PEAK", on, ACCENT).on_hover_text("Limit inter-sample peaks too").clicked() {
            cx.set("true_peak", if on { 0.0 } else { 1.0 });
        }
    });
}
