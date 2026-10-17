use super::*;
use crate::{TransportRequest, cmd};
use serde_json::json;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(noundo "transport.play", "Play", [], Some("Space"), "{from?: position}", always, |e, p| {
            if let Some(at) = position_param(e, "transport.play", p, "from")? {
                e.transport_requests.push(TransportRequest::Locate(at.max(0)));
            }
            e.transport_requests.push(TransportRequest::Play);
            Ok(json!({}))
        }),
        cmd!(noundo "transport.stop", "Stop", [], Some("Space"), "{}", always, |e, _| { e.transport_requests.push(TransportRequest::Stop); Ok(json!({})) }),
        cmd!(noundo "transport.toggle", "Play/Stop", [], Some("Space"), "{}", always, |e, _| { e.transport_requests.push(TransportRequest::TogglePlay); Ok(json!({})) }),
        cmd!(noundo "transport.record", "Record", [], Some("Cmd+Space"), "{}", always, |e, _| { e.transport_requests.push(TransportRequest::Record); Ok(json!({})) }),
        cmd!(noundo "transport.pause", "Pause", [], None, "{}", always, |e, _| { e.transport_requests.push(TransportRequest::Pause); Ok(json!({})) }),
        cmd!(noundo "transport.half_speed", "Half-Speed Playback", [], Some("Shift+Space"), "{}", always, |e, _| { e.transport_requests.push(TransportRequest::HalfSpeed); Ok(json!({})) }),
        cmd!(noundo "transport.half_speed_record", "Half-Speed Record", [], Some("Cmd+Shift+Space"), "{}", always, |e, _| { e.transport_requests.push(TransportRequest::HalfSpeedRecord); Ok(json!({})) }),
        cmd!(noundo "transport.shuttle", "Shuttle", [], None, "{speed: -4..4 (0 stops, negative is backwards)}", always, |e, p| {
            let speed = p.get("speed").and_then(Value::as_f64).ok_or_else(|| bad("transport.shuttle", "`speed` required"))?;
            if !speed.is_finite() {
                return Err(bad("transport.shuttle", "`speed` must be a number"));
            }
            let milli = (speed.clamp(-4.0, 4.0) * 1000.0).round() as i32;
            e.transport_requests.push(TransportRequest::Shuttle(milli));
            Ok(json!({"speed": f64::from(milli) / 1000.0}))
        }),
        cmd!(noundo "transport.play_selection", "Play Edit", ["Edit", "Selection"], Some("Alt+["), "{loop?: bool}", always, |e, p| {
            e.transport_requests.push(if bool_or(p, "loop", false) { TransportRequest::LoopSelection } else { TransportRequest::PlaySelection });
            Ok(json!({}))
        }),
        cmd!(noundo "transport.rtz", "Return to Zero", [], Some("Home"), "{}", always, |e, _| locate(e, 0)),
        cmd!(noundo "transport.go_to_end", "Go to End", [], Some("End"), "{}", always, |e, _| { let end = e.session().content_end(); locate(e, end) }),
        cmd!(noundo "transport.rewind", "Rewind", [], None, "{seconds?: 1}", always, |e, p| {
            let s = e.session();
            let d = s.sample_rate.samples(f64_or(p, "seconds", 1.0).clamp(0.0, 86_400.0));
            let at = (cur(e) - d).max(0);
            locate(e, at)
        }),
        cmd!(noundo "transport.fast_forward", "Fast Forward", [], None, "{seconds?: 1}", always, |e, p| {
            let s = e.session();
            let d = s.sample_rate.samples(f64_or(p, "seconds", 1.0).clamp(0.0, 86_400.0));
            let at = cur(e) + d;
            locate(e, at)
        }),
        cmd!(noundo "transport.locate", "Locate", [], None, "{at: position}", always, |e, p| {
            let at = position_param(e, "transport.locate", p, "at")?.ok_or_else(|| bad("transport.locate", "`at` required"))?;
            locate(e, at.max(0))
        }),
        cmd!(noundo "transport.status", "Transport Status", [], None, "{}", always, |e, _| Ok(json!(e.transport))),
    ]
}

fn cur(e: &Engine) -> i64 {
    if e.transport.playing { e.transport.position } else { e.session().edit.selection.start }
}

fn locate(e: &mut Engine, at: i64) -> Result<Value> {
    let s = e.session_mut();
    s.edit.playhead = at;
    if s.edit.selection.is_empty() || !s.edit.selection.contains(at) {
        s.edit.selection = openpt_time::Range::point(at);
    }
    e.transport_requests.push(TransportRequest::Locate(at));
    Ok(json!({"at": at}))
}
