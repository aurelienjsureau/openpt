use crate::SoundApp;
use crate::edit_window::{sample_at, x_of};
use crate::theme::Tokens;
use egui::{Color32, CursorIcon, Painter, Rect, Response, Shape, Stroke, Ui, pos2};
use openpt_model::{Clip, Session, Track};
use openpt_time::Samples;
use serde_json::json;

const GRAB: f32 = 5.0;

pub fn is_warp_view(track: &Track) -> bool {
    matches!(track.view.as_str(), "warp" | "analysis")
}

fn events(app: &mut SoundApp, ctx: &egui::Context, clip: &Clip) -> std::sync::Arc<Vec<Samples>> {
    let key = egui::Id::new(("warp_events", clip.id.0, format!("{:?}", clip.content), clip.length, clip.start, format!("{:?}", clip.warp)));
    if let Some(v) = ctx.data(|d| d.get_temp::<std::sync::Arc<Vec<Samples>>>(key)) {
        return v;
    }
    let found = app
        .engine
        .execute("clip.warp_events", &json!({"clip": clip.id.0}))
        .ok()
        .and_then(|v| v["events"].as_array().map(|a| a.iter().filter_map(serde_json::Value::as_i64).collect::<Vec<_>>()))
        .unwrap_or_default();
    let v = std::sync::Arc::new(found);
    ctx.data_mut(|d| d.insert_temp(key, v.clone()));
    v
}

fn markers(clip: &Clip) -> Vec<(usize, Samples)> {
    clip.warp.iter().flat_map(|w| w.markers.iter().enumerate().map(|(i, m)| (i, clip.start + m.at))).collect()
}

fn drag_id() -> egui::Id {
    egui::Id::new("warp_marker_drag")
}

pub fn draw(app: &mut SoundApp, ctx: &egui::Context, painter: &Painter, s: &Session, track: &Track, tl: Rect, lane: Rect) {
    let t = Tokens::current();
    let warp = track.view == "warp";
    let dragging: Option<(u64, usize)> = ctx.data(|d| d.get_temp(drag_id()));
    let pointer = ctx.pointer_latest_pos();
    let view = (sample_at(s, tl, tl.min.x), sample_at(s, tl, tl.max.x));
    for clip in track.clips().iter().filter(|c| c.is_audio() && c.start + c.length >= view.0 && c.start <= view.1) {
        let ev = events(app, ctx, clip);
        let col = if warp { Color32::from_white_alpha(70) } else { t.text };
        for e in ev.iter() {
            let x = x_of(s, tl, *e);
            painter.line_segment([pos2(x, lane.min.y + 12.0), pos2(x, lane.max.y)], Stroke::new(1.0, col));
        }
        if !warp {
            continue;
        }
        for (i, at) in markers(clip) {
            let live = dragging == Some((clip.id.0, i));
            let x = if live { pointer.map_or(x_of(s, tl, at), |p| p.x) } else { x_of(s, tl, at) };
            let hot = live || pointer.is_some_and(|p| lane.contains(p) && (p.x - x).abs() <= GRAB);
            let c = if hot { t.accent } else { Color32::from_gray(15) };
            painter.line_segment([pos2(x, lane.min.y), pos2(x, lane.max.y)], Stroke::new(if hot { 2.0 } else { 1.5 }, c));
            let y = lane.min.y + 1.0;
            painter.add(Shape::convex_polygon(
                vec![pos2(x - 5.0, y), pos2(x + 5.0, y), pos2(x, y + 7.0)],
                c,
                Stroke::new(1.0, Color32::from_white_alpha(120)),
            ));
        }
    }
}

pub fn lane(app: &mut SoundApp, ui: &Ui, resp: &Response, track: &Track, tl: Rect) -> bool {
    let s = app.engine.session_arc();
    let Some(p) = resp.interact_pointer_pos().or_else(|| resp.hover_pos()) else { return false };
    let y = ui.input(|i| i.pointer.press_origin()).filter(|_| resp.is_pointer_button_down_on() || resp.drag_stopped() || resp.clicked()).map_or(p.y, |o| o.y);
    let upper = y < resp.rect.center().y;
    if (s.edit.tool == openpt_model::Tool::Selector || (s.edit.tool == openpt_model::Tool::Smart && upper))
        && ui.ctx().data(|d| d.get_temp::<(u64, usize)>(drag_id())).is_none()
    {
        return false;
    }
    let at = sample_at(&s, tl, p.x).max(0);
    let clip = track.clips().iter().find(|c| c.is_audio() && at >= c.start && at < c.start + c.length).cloned();
    let gx = if resp.drag_started() { ui.input(|i| i.pointer.press_origin()).map_or(p.x, |o| o.x) } else { p.x };
    let gat = sample_at(&s, tl, gx).max(0);
    let clip = track.clips().iter().find(|c| c.is_audio() && gat >= c.start && gat < c.start + c.length).cloned().or(clip);
    let near = clip.as_ref().and_then(|c| markers(c).into_iter().find(|(_, m)| (x_of(&s, tl, *m) - gx).abs() <= GRAB).map(|(i, _)| (c.id.0, i)));
    if near.is_some() && track.view == "warp" {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if resp.drag_started()
        && near.is_none()
        && track.view == "warp"
        && let Some(c) = &clip
    {
        let ev = events(app, ui.ctx(), c);
        if let Some(e) = ev.iter().copied().filter(|e| (x_of(&s, tl, *e) - gx).abs() <= GRAB).min_by_key(|e| (x_of(&s, tl, *e) - gx).abs() as i64)
            && let Ok(v) = app.run("clip.warp_add", json!({"clip": c.id.0, "at": e}))
            && let Some(i) = v["index"].as_u64()
        {
            ui.ctx().data_mut(|d| d.insert_temp(drag_id(), (c.id.0, i as usize)));
            return true;
        }
    }
    let mut run = |id: &str, params: serde_json::Value| {
        if let Err(e) = app.run(id, params) {
            app.ui.status = e;
        }
    };
    if resp.drag_started()
        && let Some(n) = near
    {
        ui.ctx().data_mut(|d| d.insert_temp(drag_id(), n));
        return true;
    }
    if let Some((clip, index)) = ui.ctx().data(|d| d.get_temp::<(u64, usize)>(drag_id())) {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        if resp.drag_stopped() {
            ui.ctx().data_mut(|d| d.remove::<(u64, usize)>(drag_id()));
            run("clip.warp_move", json!({"clip": clip, "index": index, "to": at}));
        }
        return true;
    }
    let alt = ui.input(|i| i.modifiers.alt || i.events.iter().any(|e| matches!(e, egui::Event::PointerButton { modifiers, .. } if modifiers.alt)));
    if resp.clicked()
        && alt
        && let Some((clip, index)) = near
    {
        run("clip.warp_remove", json!({"clip": clip, "index": index}));
        return true;
    }
    if resp.clicked()
        && s.edit.tool == openpt_model::Tool::Pencil
        && track.view == "warp"
        && let Some(c) = &clip
    {
        run("clip.warp_add", json!({"clip": c.id.0, "at": at}));
        return true;
    }
    if resp.double_clicked()
        && let Some(c) = &clip
    {
        run("clip.warp_add", json!({"clip": c.id.0, "at": at}));
        return true;
    }
    resp.dragged() || resp.drag_started() || resp.drag_stopped()
}
