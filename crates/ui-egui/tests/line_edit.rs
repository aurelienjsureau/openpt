use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use openpt_model::{AutoParam, TrackId};
use openpt_ui_egui::{Services, SoundApp};
use serde_json::json;

fn harness(view: &str, tool: &str) -> (Harness<'static, SoundApp>, TrackId) {
    let mut app = SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default());
    let kick = app.engine.session().track_by_name("Kick").unwrap().id;
    app.run("edit.select", json!({"tracks": [kick.0]})).unwrap();
    app.run("track.height", json!({"height": "large"})).ok();
    app.run("track.view", json!({"view": view})).unwrap();
    app.run("edit.tool", json!({"tool": tool})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    (h, kick)
}

fn lane(h: &Harness<'_, SoundApp>, t: TrackId) -> [f32; 4] {
    let s = h.state().engine.session();
    let row = h.state().edit_layout.rows.iter().find(|r| r.0 == t.0).map(|r| r.1).unwrap();
    let tl = h.state().edit_layout.timeline;
    let height = s.track(t).unwrap().height.points();
    [row[1], row[1] + height, tl[0], tl[2]]
}

fn press(h: &mut Harness<'_, SoundApp>, at: Pos2, pressed: bool, modifiers: Modifiers) {
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers });
    h.run_steps(1);
}

fn drag(h: &mut Harness<'_, SoundApp>, from: Pos2, to: Pos2) {
    press(h, from, true, Modifiers::NONE);
    for k in 1..=10 {
        h.event(Event::PointerMoved(from + (to - from) * k as f32 / 10.0));
        h.run_steps(1);
    }
    press(h, to, false, Modifiers::NONE);
    h.run_steps(2);
}

fn volume_points(h: &Harness<'_, SoundApp>, t: TrackId) -> Vec<(i64, f32)> {
    let s = h.state().engine.session();
    s.track(t).unwrap().lane(&AutoParam::Volume).map(|l| l.points.iter().map(|p| (p.at, p.value)).collect()).unwrap_or_default()
}

#[test]
fn grabber_adds_moves_and_removes_volume_breakpoints() {
    let (mut h, kick) = harness("volume", "grabber");
    let [top, bottom, left, right] = lane(&h, kick);
    let vol = h.state().engine.session().track(kick).unwrap().mixer.volume_db;
    let y = bottom - 4.0 - (vol + 60.0) / 72.0 * (bottom - top - 8.0);
    let x = (left + right) / 2.0;
    drag(&mut h, pos2(x, y), pos2(x + 30.0, y - 20.0));
    let pts = volume_points(&h, kick);
    assert_eq!(pts.len(), 1, "one breakpoint: {pts:?}");
    assert!(pts[0].1 > vol + 3.0, "dragged up: {pts:?} from {vol}");
    let p = pos2(x + 30.0, bottom - 4.0 - (pts[0].1 + 60.0) / 72.0 * (bottom - top - 8.0));
    let s = h.state().engine.session();
    let px = left + ((pts[0].0 - s.edit.zoom.scroll) as f64 / s.edit.zoom.samples_per_px) as f32;
    let p = pos2(px, p.y);
    press(&mut h, p, true, Modifiers::ALT);
    press(&mut h, p, false, Modifiers::ALT);
    assert!(volume_points(&h, kick).is_empty(), "Alt-click removed it: {:?}", volume_points(&h, kick));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(volume_points(&h, kick).len(), 1, "undo brings the breakpoint back");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(volume_points(&h, kick).is_empty(), "adding and dragging were one undo step");
}

#[test]
fn smart_tool_edits_on_the_line_and_selects_away_from_it() {
    let (mut h, kick) = harness("volume", "smart");
    let [top, bottom, left, right] = lane(&h, kick);
    let vol = h.state().engine.session().track(kick).unwrap().mixer.volume_db;
    let y = bottom - 4.0 - (vol + 60.0) / 72.0 * (bottom - top - 8.0);
    let x = (left + right) / 2.0;
    let off = if y - top > 20.0 { top + 6.0 } else { bottom - 6.0 };
    drag(&mut h, pos2(x - 200.0, off), pos2(x - 100.0, off));
    assert!(volume_points(&h, kick).is_empty());
    let sel = h.state().engine.session().edit.selection;
    assert!(sel.end > sel.start, "a selection was made: {sel:?}");
    drag(&mut h, pos2(x, y), pos2(x, y + 15.0));
    let pts = volume_points(&h, kick);
    assert_eq!(pts.len(), 1, "{pts:?}");
    assert!(pts[0].1 < vol - 2.0, "dragged down: {pts:?}");
}

#[test]
fn grabber_edits_the_clip_gain_line() {
    let (mut h, kick) = harness("clip_gain", "grabber");
    let [_, bottom, left, _] = lane(&h, kick);
    let s = h.state().engine.session();
    let clip = s.track(kick).unwrap().clips().first().cloned().unwrap();
    let spp = s.edit.zoom.samples_per_px;
    let xc = |at: i64| left + ((at - s.edit.zoom.scroll) as f64 / spp) as f32;
    let x = (xc(clip.start) + xc(clip.end())) / 2.0;
    assert!(x > left + 10.0, "clip visible");
    let mut y_line = None;
    for k in 0..200 {
        let y = bottom - 2.0 - k as f32 * 0.5;
        h.event(Event::PointerMoved(pos2(x, y)));
        h.run_steps(1);
        if h.output().platform_output.cursor_icon == egui::CursorIcon::PointingHand {
            y_line = Some(y);
            break;
        }
    }
    let y = y_line.expect("the gain line shows the add cursor");
    drag(&mut h, pos2(x, y), pos2(x, y - 25.0));
    let s = h.state().engine.session();
    let c = s.find_clip(clip.id).unwrap().1;
    assert!(!c.gain_env.is_empty(), "a gain breakpoint: {:?} {}", c.gain_env, c.gain_db);
    let peak = c.gain_env.iter().map(|p| c.gain_db + p.1).fold(f32::MIN, f32::max);
    assert!(peak > 1.0, "raised: {:?}", c.gain_env);
}

#[test]
fn with_a_selection_the_line_moves_as_a_plateau() {
    let (mut h, kick) = harness("volume", "grabber");
    let [top, bottom, left, right] = lane(&h, kick);
    let s = h.state().engine.session();
    let (vol, scroll, spp) = (s.track(kick).unwrap().mixer.volume_db, s.edit.zoom.scroll, s.edit.zoom.samples_per_px);
    let at = |x: f32| scroll + (f64::from(x - left) * spp) as i64;
    let (x0, x1) = (left + (right - left) * 0.4, left + (right - left) * 0.6);
    h.state_mut().run("edit.select", json!({"tracks": [kick.0], "start": at(x0), "end": at(x1)})).unwrap();
    h.run_steps(2);
    let y = bottom - 4.0 - (vol + 60.0) / 72.0 * (bottom - top - 8.0);
    drag(&mut h, pos2((x0 + x1) / 2.0, y), pos2((x0 + x1) / 2.0, y - 20.0));
    let pts = volume_points(&h, kick);
    let (a, b) = (at(x0), at(x1));
    assert_eq!(pts.iter().map(|p| p.0).collect::<Vec<_>>(), vec![a - 1, a, b, b + 1], "{pts:?}");
    assert!((pts[0].1 - vol).abs() < 0.01 && (pts[3].1 - vol).abs() < 0.01, "outside unchanged: {pts:?}");
    assert!(pts[1].1 > vol + 2.0 && (pts[1].1 - pts[2].1).abs() < 0.01, "a raised plateau: {pts:?}");
    h.state_mut().run("edit.select", json!({"tracks": [kick.0], "start": 0})).unwrap();
    h.state_mut().run("automation.clear", json!({"tracks": [kick.0], "param": "volume", "start": 0, "end": 1_000_000_000})).unwrap();
    h.run_steps(2);
    drag(&mut h, pos2((x0 + x1) / 2.0, y), pos2((x0 + x1) / 2.0, y - 20.0));
    assert_eq!(volume_points(&h, kick).len(), 1, "{:?}", volume_points(&h, kick));
}

#[test]
fn with_a_selection_the_clip_gain_line_moves_as_a_plateau() {
    let (mut h, kick) = harness("clip_gain", "grabber");
    let [_, bottom, left, _] = lane(&h, kick);
    let s = h.state().engine.session();
    let clip = s.track(kick).unwrap().clips().first().cloned().unwrap();
    let (scroll, spp) = (s.edit.zoom.scroll, s.edit.zoom.samples_per_px);
    let xc = |at: i64| left + ((at - scroll) as f64 / spp) as f32;
    let (a, b) = (clip.start + clip.length / 3, clip.start + 2 * clip.length / 3);
    h.state_mut().run("edit.select", json!({"tracks": [kick.0], "start": a, "end": b})).unwrap();
    h.run_steps(2);
    let x = (xc(a) + xc(b)) / 2.0;
    let mut y_line = None;
    for k in 0..400 {
        let y = bottom - 2.0 - k as f32 * 0.5;
        h.event(Event::PointerMoved(pos2(x, y)));
        h.run_steps(1);
        if h.output().platform_output.cursor_icon == egui::CursorIcon::PointingHand {
            y_line = Some(y - 4.0);
            break;
        }
    }
    let y = y_line.expect("gain line");
    drag(&mut h, pos2(x, y), pos2(x, y - 60.0));
    let s = h.state().engine.session();
    let c = s.find_clip(clip.id).unwrap().1;
    let at = |o: i64| c.gain_db_at(o);
    assert!(at((a + b) / 2 - c.start) > 1.0, "inside raised: {:?}", c.gain_env);
    assert!(at(a - c.start - 10).abs() < 0.01 && at(b - c.start + 10).abs() < 0.01, "outside unchanged: {:?}", c.gain_env);
}
