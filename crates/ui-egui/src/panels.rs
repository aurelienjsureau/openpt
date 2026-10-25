use crate::SoundApp;
use crate::theme::{Tokens, bold, mono, regular, rgb};
use egui::{Align2, Color32, Rect, Sense, Stroke, Ui, pos2, vec2};
use openpt_model::{ClipContent, ClipId, SourceId};
use serde_json::json;

fn drag_track(app: &mut SoundApp, ui: &Ui, row: Rect, id: u64, dropped: bool) {
    let s = app.engine.session();
    let (Some(from), Some(p)) = (s.track_index(openpt_model::TrackId(id)), ui.ctx().pointer_latest_pos()) else { return };
    let spacing = ui.spacing().item_spacing.y;
    let pitch = row.height() + spacing;
    let gap = (from as f32 + ((p.y - row.center().y) / pitch).floor() + 1.0).clamp(0.0, s.tracks.len() as f32);
    if dropped {
        crate::edit_window::move_tracks_to(app, openpt_model::TrackId(id), gap as usize);
    } else {
        let y = row.min.y + (gap - from as f32) * pitch - spacing / 2.0;
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let clip = ui.clip_rect();
        let push = if p.y < clip.min.y + 20.0 {
            clip.min.y + 20.0 - p.y
        } else if p.y > clip.max.y - 20.0 {
            clip.max.y - 20.0 - p.y
        } else {
            0.0
        };
        if push != 0.0 {
            ui.scroll_with_delta(vec2(0.0, (push * 0.5).clamp(-30.0, 30.0)));
            ui.ctx().request_repaint();
        }
        ui.ctx()
            .layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("track_list_reorder")))
            .line_segment([pos2(row.min.x, y), pos2(row.max.x, y)], Stroke::new(2.0, Tokens::current().accent));
    }
}

fn panel_header(ui: &mut Ui, title: &str) -> Rect {
    let t = Tokens::current();
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.panel_bg2);
    ui.painter().text(pos2(r.min.x + 8.0, r.center().y), Align2::LEFT_CENTER, title, bold(11.5), t.header_text);
    ui.painter().line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0, t.border));
    r
}

fn header_tools(ui: &mut Ui, head: Rect, add: impl FnOnce(&mut Ui)) {
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(vec2(4.0, 3.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    add(&mut child);
}

const GUTTER_W: f32 = 10.0;

pub fn tracks_and_groups(app: &mut SoundApp, ui: &mut Ui) {
    let _prof = crate::prof::scope("tracks_list");
    let t = Tokens::current();
    let full = ui.max_rect();
    let gutter = full.with_min_x(full.max.x - GUTTER_W);
    ui.painter().rect_filled(gutter, 0.0, t.panel_bg.lerp_to_gamma(t.text_dim, 0.15));
    ui.painter().vline(gutter.min.x + 0.5, full.y_range(), Stroke::new(1.0, t.border));
    ui.painter().vline(gutter.max.x - 0.5, full.y_range(), Stroke::new(1.0, t.border));
    let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(full.with_max_x(gutter.min.x)));
    let total = ui.available_height();
    let header = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 22.0));
    panel_header(ui, "TRACKS");
    let plus = Rect::from_center_size(pos2(header.max.x - 13.0, header.center().y), vec2(18.0, 18.0));
    let tip = format!("New Tracks… ({})", crate::shortcuts::shortcut_label("Cmd+Shift+N", ui.ctx().os().is_mac()));
    if crate::widgets::icon_button(&mut ui.new_child(egui::UiBuilder::new().max_rect(plus)), plus.size(), "plus", false, &tip).clicked() {
        let _ = app.dialogs.open_for_command(&app.engine, "track.new");
    }
    let tracks: Vec<(u64, String, bool, [u8; 3], bool, usize, Option<bool>)> = {
        let s = app.engine.session();
        let chain = |x: &openpt_model::Track| {
            let (mut depth, mut open, mut cur) = (0, true, x.folder);
            while let Some(f) = cur.and_then(|f| s.track(f)).filter(|_| depth < 64) {
                depth += 1;
                open &= f.folder_open;
                cur = f.folder;
            }
            (depth, open)
        };
        s.tracks
            .iter()
            .filter_map(|x| {
                let (depth, open) = chain(x);
                open.then(|| {
                    (
                        x.id.0,
                        x.name.clone(),
                        !x.hidden,
                        x.color,
                        s.edit.selected_tracks.contains(&x.id),
                        depth,
                        x.is_folder().then_some(x.folder_open),
                    )
                })
            })
            .collect()
    };
    let hidden = egui::scroll_area::ScrollBarVisibility::AlwaysHidden;
    let lists =
        egui::ScrollArea::vertical().id_salt("tracks_scroll").max_height(total * 0.62).auto_shrink([false, false]).scroll_bar_visibility(hidden);
    let out = lists.show(ui, |ui| {
        for (id, name, shown, color, sel, depth, folder) in tracks {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 17.0), Sense::click_and_drag());
            if sel {
                ui.painter().rect_filled(r, 0.0, t.row_selected);
            }
            let dot = Rect::from_center_size(pos2(r.min.x + 10.0, r.center().y), vec2(8.0, 8.0));
            let dresp = ui.interact(dot.expand(3.0), ui.id().with(("vis", id)), Sense::click());
            ui.painter().circle(dot.center(), 3.5, if shown { t.visible_dot } else { Color32::TRANSPARENT }, Stroke::new(1.0, t.text_dim));
            if dresp.clicked() {
                let _ = app.run("track.hide", json!({"tracks": [id], "hidden": shown}));
            }
            let ind = depth as f32 * 12.0;
            if let Some(open) = folder {
                let tri = Rect::from_center_size(pos2(r.min.x + 22.0 + ind, r.center().y), vec2(10.0, 10.0));
                let c = tri.center();
                let pts = if open {
                    vec![c + vec2(-4.0, -2.5), c + vec2(4.0, -2.5), c + vec2(0.0, 3.0)]
                } else {
                    vec![c + vec2(-2.5, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.5, 4.0)]
                };
                ui.painter().add(egui::Shape::convex_polygon(pts, t.text_dim, Stroke::NONE));
                if ui.interact(tri.expand(2.0), ui.id().with(("folder-open", id)), Sense::click()).clicked() {
                    let _ = app.run("track.folder_toggle", json!({"track": id}));
                }
            }
            let ind = ind + if folder.is_some() { 12.0 } else { 0.0 };
            ui.painter().rect_filled(Rect::from_min_size(pos2(r.min.x + 20.0 + ind, r.min.y + 3.0), vec2(4.0, 11.0)), 0.0, rgb(color));
            ui.painter().text(
                pos2(r.min.x + 30.0 + ind, r.center().y),
                Align2::LEFT_CENTER,
                &name,
                regular(11.5),
                if shown { t.text } else { t.text_dim },
            );
            if resp.clicked() {
                crate::edit_window::click_track(app, openpt_model::TrackId(id), ui.input(|i| i.modifiers), |_, _| true);
                app.edit_layout.reveal(openpt_model::TrackId(id));
            }
            if resp.dragged() || resp.drag_stopped() {
                drag_track(app, ui, r, id, resp.drag_stopped());
            }
        }
    });
    gutter_scroll(ui, gutter, &out);
    ui.add_space(4.0);
    let head = panel_header(ui, "GROUPS");
    header_tools(ui, head, |ui| focus_button(app, ui, "groups", "Groups List Keyboard Focus (type a group letter to enable or disable it)"));
    let groups: Vec<(u64, char, String, bool, [u8; 3])> =
        app.engine.session().groups.iter().map(|g| (g.id.0, g.letter, g.name.clone(), g.active, g.color)).collect();
    let out = egui::ScrollArea::vertical().id_salt("groups_scroll").auto_shrink([false, false]).scroll_bar_visibility(hidden).show(ui, |ui| {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 17.0), Sense::hover());
        ui.painter().text(pos2(r.min.x + 30.0, r.center().y), Align2::LEFT_CENTER, "<ALL>", regular(11.5).clone(), t.text);
        let all = ui.interact(r, ui.id().with("group_all"), Sense::click());
        if all.double_clicked() {
            let _ = app.run("edit.select_all", json!({}));
        }
        all.context_menu(|ui| {
            if ui.button("New Group…").clicked() {
                let _ = app.dialogs.open_for_command(&app.engine, "track.group");
                ui.close();
            }
            let suspended = app.engine.session().edit.flag("groups.suspended");
            if ui
                .add(
                    egui::Button::new("Suspend All Groups")
                        .selected(suspended)
                        .shortcut_text(crate::shortcuts::shortcut_label("Cmd+Shift+G", ui.ctx().os().is_mac())),
                )
                .clicked()
            {
                let _ = app.run("mix.suspend_groups", json!({}));
                ui.close();
            }
        });
        let suspended = app.engine.session().edit.flag("groups.suspended");
        for (id, letter, name, active, color) in groups {
            let active = active && !suspended;
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 17.0), Sense::click());
            if active {
                ui.painter().rect_filled(Rect::from_min_size(pos2(r.min.x + 4.0, r.min.y + 2.0), vec2(14.0, 13.0)), 1.0, rgb(color));
            }
            ui.painter().text(
                pos2(r.min.x + 11.0, r.center().y),
                Align2::CENTER_CENTER,
                letter.to_string(),
                bold(10.0),
                if active { t.text_dark } else { t.text_dim },
            );
            ui.painter().text(pos2(r.min.x + 30.0, r.center().y), Align2::LEFT_CENTER, &name, regular(11.5), t.text);
            if resp.clicked() {
                let _ = app.run("track.group_toggle", json!({"group": id}));
            }
            let members: Vec<u64> =
                app.engine.session().groups.iter().find(|g| g.id.0 == id).map(|g| g.members.iter().map(|m| m.0).collect()).unwrap_or_default();
            if resp.double_clicked() {
                let _ = app.run("edit.select", json!({"tracks": members, "exact": true}));
            }
            resp.context_menu(|ui| {
                if ui.button("Modify Group...").clicked() {
                    let s = app.engine.session_arc();
                    app.dialogs.open_modify_group(&s, id);
                    ui.close();
                }
                if ui.button("Select Tracks in Group").clicked() {
                    let _ = app.run("edit.select", json!({"tracks": members, "exact": true}));
                    ui.close();
                }
                if ui.button(if active { "Disable" } else { "Enable" }).clicked() {
                    let _ = app.run("track.group_toggle", json!({"group": id}));
                    ui.close();
                }
                if ui.button("Delete Group").clicked() {
                    let _ = app.run("track.ungroup", json!({"group": id}));
                    ui.close();
                }
            });
        }
    });
    gutter_scroll(ui, gutter, &out);
}

fn gutter_scroll<R>(ui: &Ui, gutter: Rect, out: &egui::scroll_area::ScrollAreaOutput<R>) {
    let t = Tokens::current();
    let (view, content) = (out.inner_rect.height(), out.content_size.y);
    if content <= view + 0.5 {
        return;
    }
    let bar = Rect::from_x_y_ranges(gutter.x_range(), out.inner_rect.y_range());
    let (max, h) = (content - view, (bar.height() * view / content).max(20.0));
    let mut state = out.state;
    let thumb = Rect::from_min_size(pos2(bar.min.x + 2.0, bar.min.y + (bar.height() - h) * state.offset.y / max), vec2(bar.width() - 4.0, h));
    let resp = ui.interact(bar, out.id.with("gutter"), Sense::click_and_drag());
    ui.painter().rect_filled(thumb, 3.0, if resp.hovered() || resp.dragged() { t.button_hi.lerp_to_gamma(Color32::WHITE, 0.1) } else { t.button_hi });
    let to = if resp.dragged() {
        Some(state.offset.y + resp.drag_delta().y * max / (bar.height() - h).max(1.0))
    } else if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.clicked()) {
        let page = if p.y < thumb.min.y {
            -view
        } else if p.y > thumb.max.y {
            view
        } else {
            0.0
        };
        Some(state.offset.y + page)
    } else {
        None
    };
    if let Some(y) = to {
        state.offset.y = y.clamp(0.0, max);
        state.store(ui.ctx(), out.id);
    }
}

pub(crate) fn clip_list_items(app: &SoundApp) -> Vec<(u64, String, bool, [u8; 3], bool)> {
    let s = app.engine.session();
    let [audio, midi, groups, auto] = app.ui.clip_list_show;
    let sel_clips: std::collections::HashSet<openpt_model::ClipId> = s.edit.selected_clips.iter().copied().collect();
    let mut v = Vec::new();
    let renders: std::collections::HashSet<openpt_model::SourceId> =
        s.tracks.iter().flat_map(|t| t.playlists.iter()).flat_map(|p| &p.clips).filter_map(|c| c.warp.as_ref().and_then(|w| w.rendered)).collect();
    if audio {
        for src in s.sources.iter().filter(|x| !renders.contains(&x.id)) {
            v.push((src.id.0, format!("{} ({}ch)", src.name, src.channels), true, [150, 150, 150], s.edit.selected_sources.contains(&src.id)));
        }
    }
    for tr in &s.tracks {
        for c in tr.clips() {
            let show = match &c.content {
                _ if c.group.is_some() => groups,
                ClipContent::Midi { .. } => midi,
                ClipContent::Audio { source, offset } => {
                    let whole = *offset == 0 && s.sources.iter().find(|x| x.id == *source).is_some_and(|x| x.frames == c.length as u64);
                    audio && (whole || auto)
                }
                ClipContent::Video { .. } => true,
            };
            if show {
                v.push((c.id.0, c.name.clone(), false, c.color.unwrap_or(tr.color), sel_clips.contains(&c.id)));
            }
        }
    }
    v
}

fn focus_button(app: &mut SoundApp, ui: &mut Ui, focus: &str, tip: &str) {
    let t = Tokens::current();
    let on = app.engine.session().edit.keyboard_focus == focus;
    if crate::widgets::text_toggle(ui, vec2(28.0, 15.0), "a-z", on, t.accent, tip).clicked() {
        let _ = app.run("options.keyboard_focus", json!({"focus": if on { "none" } else { focus }}));
    }
}

pub fn clip_list(app: &mut SoundApp, ui: &mut Ui) {
    let _prof = crate::prof::scope("clip_list");
    let t = Tokens::current();
    let head = panel_header(ui, "CLIPS");
    header_tools(ui, head, |ui| {
        ui.menu_button(egui::RichText::new("Show").font(regular(10.0)), |ui| {
            for (i, label) in ["Audio", "MIDI", "Clip Groups", "Auto-Created"].into_iter().enumerate() {
                ui.checkbox(&mut app.ui.clip_list_show[i], label);
            }
        });
        focus_button(app, ui, "clips", "Clip List Keyboard Focus (type a name to select a clip)");
    });
    let items = clip_list_items(app);
    let fkey = egui::Id::new("clip_filter");
    let mut filter: String = ui.ctx().memory(|m| m.data.get_temp(fkey)).unwrap_or_default();
    let find = ui.add(egui::TextEdit::singleline(&mut filter).hint_text("🔍 Find clips").desired_width(f32::INFINITY));
    if std::mem::take(&mut app.focus_clip_find) {
        find.request_focus();
    }
    ui.ctx().memory_mut(|m| m.data.insert_temp(fkey, filter.clone()));
    let needle = filter.to_lowercase();
    let items: Vec<_> = items.into_iter().filter(|(_, n, _, _, _)| needle.is_empty() || n.to_lowercase().contains(&needle)).collect();
    let files: Vec<u64> = items.iter().filter(|i| i.2).map(|i| i.0).collect();
    let chosen: Vec<u64> = items.iter().filter(|i| i.2 && i.4).map(|i| i.0).collect();
    let anchor_id = egui::Id::new("clip_list_anchor");
    let row_h = 17.0;
    egui::ScrollArea::vertical().id_salt("clips_scroll").auto_shrink([false, false]).show_rows(ui, row_h, items.len(), |ui, rows| {
        for (id, name, whole, color, selected) in items[rows].iter().cloned() {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 17.0), if whole { Sense::click_and_drag() } else { Sense::click() });
            resp.context_menu(|ui| {
                if whole {
                    if ui.button("Place on New Track").clicked() {
                        let _ = app.run("clip.place_source", json!({"source": id}));
                        ui.close();
                    }
                    let sel_track = app.engine.session().edit.selected_tracks.first().map(|t| t.0);
                    if let Some(t) = sel_track
                        && ui.button("Place on Selected Track").clicked()
                    {
                        let _ = app.run("clip.place_source", json!({"source": id, "track": t}));
                        ui.close();
                    }
                    ui.separator();
                }
                if ui.button("Clear…").clicked() {
                    let (clips, sources) = if whole { (vec![], vec![SourceId(id)]) } else { (vec![ClipId(id)], vec![]) };
                    app.dialogs.open_clear(&app.engine, clips, sources);
                    ui.close();
                }
            });
            if whole {
                resp.dnd_set_drag_payload(crate::DragSource(if selected { chosen.clone() } else { vec![id] }));
                if resp.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
            }
            if selected {
                ui.painter().rect_filled(r, 0.0, t.row_selected);
            }
            ui.painter().rect_filled(Rect::from_min_size(pos2(r.min.x + 6.0, r.min.y + 4.0), vec2(9.0, 9.0)), 1.0, rgb(color));
            ui.painter().with_clip_rect(r).text(
                pos2(r.min.x + 22.0, r.center().y),
                Align2::LEFT_CENTER,
                &name,
                if whole { bold(11.0) } else { regular(11.0) },
                t.text,
            );
            if resp.clicked() {
                let m = ui.input(|i| i.modifiers);
                let anchor: Option<u64> = ui.data(|d| d.get_temp(anchor_id));
                let sources: Vec<u64> = match (whole, anchor.and_then(|a| files.iter().position(|f| *f == a)), files.iter().position(|f| *f == id)) {
                    (true, Some(a), Some(b)) if m.shift => files[a.min(b)..=a.max(b)].to_vec(),
                    (true, _, _) if m.command => {
                        let mut s = chosen.clone();
                        if selected {
                            s.retain(|x| *x != id)
                        } else {
                            s.push(id)
                        }
                        s
                    }
                    _ => vec![id],
                };
                if !m.shift {
                    ui.data_mut(|d| d.insert_temp(anchor_id, id));
                }
                let _ = app.run("edit.select", if whole { json!({"sources": sources}) } else { json!({"clips": [id]}) });
            }
        }
    });
}

pub fn floating(app: &mut SoundApp, ctx: &egui::Context) {
    transport_window(app, ctx);
    big_counter(app, ctx);
    plugin_windows(app, ctx);
    crate::mix_window::send_window(app, ctx);
    about(app, ctx);
    session_info(app, ctx);
}

fn transport_window(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_transport;
    if !open {
        return;
    }
    let t = Tokens::current();
    egui::Window::new("Transport").open(&mut open).resizable(false).default_pos(pos2(400.0, 500.0)).show(ctx, |ui| {
        let flags = app.engine.session().edit.flags.clone();
        let expanded = flags.contains("view.transport.expanded");
        ui.horizontal(|ui| {
            for (icon, cmd) in [
                ("rtz", "transport.rtz"),
                ("rewind", "transport.rewind"),
                ("stop", "transport.stop"),
                ("play", "transport.play"),
                ("ffwd", "transport.fast_forward"),
                ("end", "transport.go_to_end"),
                ("record", "transport.record"),
            ] {
                let resp = crate::widgets::icon_button(
                    ui,
                    vec2(34.0, 26.0),
                    icon,
                    (cmd == "transport.play" && app.is_playing()) || (cmd == "transport.record" && app.engine.transport.recording),
                    cmd,
                );
                if cmd == "transport.record"
                    && let Some(why) = app.record_hint()
                {
                    egui::Tooltip::for_widget(&resp).show(|ui| ui.label(why));
                }
                if resp.clicked() {
                    let _ = app.run(cmd, json!({}));
                }
            }
            if flags.contains("view.transport.output_meters") {
                let (r, _) = ui.allocate_exact_size(vec2(22.0, 26.0), Sense::hover());
                let m = app.main_meter;
                crate::widgets::meter(ui, Rect::from_min_size(r.min, vec2(10.0, 26.0)), m.shown(0), m.hold[0], m.clip);
                crate::widgets::meter(ui, Rect::from_min_size(pos2(r.min.x + 12.0, r.min.y), vec2(10.0, 26.0)), m.shown(1), m.hold[1], m.clip);
            }
        });
        let s = app.engine.session();
        let fmt = |x: i64| s.format_position(x, s.edit.main_counter);
        if flags.contains("view.transport.counters") || expanded || flags.is_empty() {
            ui.label(egui::RichText::new(fmt(app.position())).font(mono(20.0)).color(t.counter_text));
        }
        if expanded {
            let e = s.edit.clone();
            egui::Grid::new("tx_exp").num_columns(4).show(ui, |ui| {
                ui.label("Start");
                ui.label(egui::RichText::new(fmt(e.selection.start)).font(mono(11.0)));
                ui.label("Pre-roll");
                ui.label(egui::RichText::new(format!("{:.2} s", s.sample_rate.seconds(e.pre_roll))).font(mono(11.0)));
                ui.end_row();
                ui.label("End");
                ui.label(egui::RichText::new(fmt(e.selection.end)).font(mono(11.0)));
                ui.label("Post-roll");
                ui.label(egui::RichText::new(format!("{:.2} s", s.sample_rate.seconds(e.post_roll))).font(mono(11.0)));
                ui.end_row();
            });
        }
        if flags.contains("view.transport.midi_controls") || expanded {
            let (click, countoff, merge, loop_rec, prepost) = {
                let e = &app.engine.session().edit;
                (e.click, e.countoff, e.midi_merge, e.loop_record, e.pre_post_roll)
            };
            ui.horizontal(|ui| {
                for (label, on, cmd) in [
                    ("Click", click, "options.click"),
                    ("Count Off", countoff, "options.countoff"),
                    ("MIDI Merge", merge, "options.midi_merge"),
                    ("Loop Rec", loop_rec, "options.loop_record"),
                    ("Pre/Post", prepost, "options.pre_post_roll"),
                ] {
                    if ui.selectable_label(on, label).clicked() {
                        let _ = app.run(cmd, json!({}));
                    }
                }
            });
            let tick = app.engine.session().tempo.samples_to_ticks(app.position(), app.engine.session().sample_rate);
            let bpm = app.engine.session().tempo.tempo_at_tick(tick);
            let m = app.engine.session().tempo.meter_at_tick(tick);
            ui.label(egui::RichText::new(format!("♩ = {bpm:.2}   {}/{}", m.numerator, m.denominator)).font(mono(12.0)).color(t.counter_text));
        }
    });
    app.ui.show_transport = open;
}

fn big_counter(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_big_counter;
    if !open {
        return;
    }
    egui::Window::new("Big Counter").open(&mut open).default_size(vec2(520.0, 110.0)).show(ctx, |ui| {
        let s = app.engine.session();
        let txt = s.format_position(app.position(), s.edit.main_counter);
        let r = ui.available_rect_before_wrap();
        let t = Tokens::current();
        ui.painter().rect_filled(r, 4.0, t.big_counter_bg);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, txt, mono((r.height() * 0.6).clamp(20.0, 120.0)), t.counter_text);
    });
    app.ui.show_big_counter = open;
}

pub(crate) fn memory_locations(app: &mut SoundApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        if ui.button("+ Marker").clicked() {
            let _ = app.run("markers.add", json!({}));
        }
        if ui.button("+ Selection").clicked() {
            let _ = app.run("markers.add", json!({"kind": "selection"}));
        }
    });
    let rows: Vec<(u32, String, String, String, String)> = {
        let s = app.engine.session();
        let f = |x: i64| s.format_position(x, s.edit.main_counter);
        s.markers
            .iter()
            .map(|m| {
                let kind = match m.kind {
                    openpt_model::MarkerKind::Marker => "marker",
                    openpt_model::MarkerKind::Selection => "selection",
                    openpt_model::MarkerKind::None => "none",
                };
                let len = if m.kind == openpt_model::MarkerKind::Selection { f(m.end - m.start) } else { String::new() };
                (m.number, m.name.clone(), f(m.start), len, kind.to_string())
            })
            .collect()
    };
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("mem_grid").striped(true).num_columns(6).show(ui, |ui| {
            for h in ["#", "Name", "Location", "Length", "Type", ""] {
                ui.label(egui::RichText::new(h).strong());
            }
            ui.end_row();
            for (n, name, loc, len, kind) in rows {
                if ui.button(n.to_string()).on_hover_text("Recall").clicked() {
                    let _ = app.run("markers.recall", json!({"number": n}));
                }
                let key = egui::Id::new(("mem_name", n));
                let mut edit: String = ui.ctx().memory(|m| m.data.get_temp(key)).unwrap_or_else(|| name.clone());
                let r = ui.add(egui::TextEdit::singleline(&mut edit).desired_width(140.0));
                if r.changed() {
                    ui.ctx().memory_mut(|m| m.data.insert_temp(key, edit.clone()));
                }
                if r.lost_focus() && edit != name {
                    let _ = app.run("markers.edit", json!({"number": n, "rename": edit}));
                    ui.ctx().memory_mut(|m| m.data.remove::<String>(key));
                }
                ui.label(egui::RichText::new(loc).font(mono(11.0)));
                ui.label(egui::RichText::new(len).font(mono(11.0)));
                ui.label(kind);
                if ui.small_button("✕").on_hover_text("Delete").clicked() {
                    let _ = app.run("markers.delete", json!({"number": n}));
                }
                ui.end_row();
            }
        });
    });
}

pub(crate) fn undo_history(app: &mut SoundApp, ui: &mut Ui) {
    let hist = app.engine.undo_history();
    let n = hist.len();
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (i, h) in hist.iter().enumerate() {
            if ui.selectable_label(i + 1 == n, format!("{}  {h}", i + 1)).clicked() {
                for _ in 0..(n - i - 1) {
                    let _ = app.run("edit.undo", json!({}));
                }
            }
        }
        if let Some(r) = app.engine.redo_label() {
            ui.label(egui::RichText::new(format!("(redo) {r}")).italics().color(Tokens::current().text_dim));
        }
    });
}

fn plugin_windows(app: &mut SoundApp, ctx: &egui::Context) {
    crate::plugin_ui::automation_dialog(app, ctx);
    let wins = app.ui.plugin_windows.clone();
    let mut keep = Vec::new();
    for (tid, slot) in wins {
        let instrument = slot == openpt_mix::INSTRUMENT_SLOT;
        let Some((tname, ins)) = app.engine.session().track(tid).and_then(|t| {
            let ins = if instrument { t.instrument.clone() } else { t.mixer.inserts.get(slot).cloned().flatten() };
            ins.map(|i| (t.name.clone(), i))
        }) else {
            continue;
        };
        let Some(info) = crate::mix_window::plugin_info(&ins.plugin) else { continue };
        let place = if instrument { "instrument".to_string() } else { char::from(b'a'.saturating_add(slot as u8)).to_string() };
        let mut open = true;
        let builtin = !instrument && !openpt_mix::is_third_party(&ins.plugin);
        let window = egui::Window::new(format!("{tname} · {place} · {}", info.name))
            .id(egui::Id::new((if builtin { "plugin-panel" } else { "plugin" }, tid.0, slot)))
            .open(&mut open)
            .default_width(340.0)
            .default_pos(egui::pos2(ctx.content_rect().width() - 380.0, 120.0));
        let window = if builtin { window.auto_sized() } else { window };
        let shown = window.show(ctx, |ui| {
            if builtin {
                ui.set_max_width(crate::plugin_ui::panel_width(info));
                crate::plugin_ui::header(app, ui, tid, slot, info, &ins, |app, ui| preset_menu(app, ui, tid, slot, info, &ins));
                ui.add_space(4.0);
                let mut changes: Vec<(String, f32)> = Vec::new();
                let room = (ui.ctx().content_rect().max.y - ui.cursor().min.y - 24.0).max(120.0);
                egui::ScrollArea::vertical().max_height(room).show(ui, |ui| {
                    let scope = openpt_dsp::scope::find(tid.0, slot);
                    let salt = egui::Id::new(("open-plugin", tid.0, slot));
                    if !crate::open_ui::panel(ui, info, &ins.params, scope, salt, &mut |id, v| changes.push((id.to_string(), v))) {
                        let gr = app.meters.get(&tid).map_or(0.0, |m| m.gr);
                        crate::plugin_ui::panel(ui, info, &ins, gr, |id, v| changes.push((id.to_string(), v)));
                    }
                });
                for (id, v) in changes {
                    let _ = app.engine.execute_merged(
                        "mix.insert_param",
                        &json!({"track": tid.0, "slot": slot, "param": id, "value": v}),
                        &format!("param:{}:{slot}:{id}", tid.0),
                    );
                    app.touch_param(tid, slot, &id, v);
                }
                return;
            }
            ui.horizontal(|ui| {
                if !instrument {
                    ui.label(egui::RichText::new(&ins.preset).color(Tokens::current().text_dim));
                    let label = if ins.bypass { "BYPASSED" } else { "Bypass" };
                    if ui.button(label).clicked() {
                        let _ = app.run("mix.insert_bypass", json!({"track": tid.0, "slot": slot}));
                    }
                    ui.menu_button("Presets ▾", |ui| preset_menu(app, ui, tid, slot, info, &ins));
                    if ui.button("Auto").on_hover_text("Plug-in Automation: choose what can be automated").clicked() {
                        crate::plugin_ui::open_automation_dialog(ui.ctx(), tid, slot);
                    }
                }
                if openpt_mix::is_third_party(&ins.plugin) {
                    let open = app.player.as_ref().is_some_and(|p| p.editor_open(tid, slot));
                    let available = app.player.as_ref().is_some_and(|p| p.has_editor(tid, slot));
                    let label = if open { "Close Plugin Editor" } else { "Open Plugin Editor" };
                    let resp = ui.add_enabled(available, egui::Button::new(label));
                    let resp =
                        if available { resp } else { resp.on_disabled_hover_text("This plugin has no editor of its own, or it is not loaded") };
                    if resp.clicked()
                        && let Some(p) = &app.player
                    {
                        if open {
                            p.close_editor(tid, slot);
                        } else if let Err(e) = p.open_editor(tid, slot) {
                            app.ui.status = format!("{}: {e}", info.name);
                        }
                    }
                }
            });
            if !instrument && crate::plugin_ui::takes_key(&ins.plugin) {
                ui.horizontal(|ui| crate::plugin_ui::key_input(app, ui, tid, slot, &ins));
            }
            if instrument && openpt_mix::is_third_party(&ins.plugin) {
                ui.label(egui::RichText::new("Load and edit sounds in the plugin's own editor.").color(Tokens::current().text_dim));
                return;
            }
            if instrument && ins.plugin == crate::sampler_window::SAMPLER {
                crate::sampler_window::sample_strip(app, ui, tid, &ins);
                ui.add_space(4.0);
            }
            if info.id == "eq_7band" || info.id == "eq_1band" {
                eq_curve(ui, info.id, &ins);
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new(("pg", tid.0, slot)).num_columns(3).show(ui, |ui| {
                    for p in info.params {
                        let v = ins.params.get(p.id).copied().unwrap_or(p.default);
                        let name = ui.add(egui::Label::new(p.name).sense(egui::Sense::click()));
                        if !instrument {
                            name.on_hover_text("Right-click: show its automation").context_menu(|ui| {
                                if ui.button("Show Automation Lane").clicked() {
                                    let view = format!("plugin:{}:{}", char::from(b'a'.saturating_add(slot as u8)), p.id);
                                    let _ = app.run("track.view", json!({"tracks": [tid.0], "view": view}));
                                    ui.close();
                                }
                            });
                        }
                        let mut x = v;
                        let resp = if !p.choices.is_empty() {
                            let mut idx = x.round().max(0.0) as usize;
                            let r = egui::ComboBox::from_id_salt(("pc", tid.0, slot, p.id))
                                .selected_text(p.choices.get(idx).copied().unwrap_or("?"))
                                .show_ui(ui, |ui| {
                                    for (i, c) in p.choices.iter().enumerate() {
                                        ui.selectable_value(&mut idx, i, *c);
                                    }
                                });
                            x = idx as f32;
                            r.response
                        } else {
                            let mut sl = egui::Slider::new(&mut x, p.min..=p.max).show_value(false);
                            if instrument && crate::sampler_window::whole_number(&ins.plugin, p.id) {
                                sl = sl.integer();
                            }
                            if p.taper == openpt_dsp::Taper::Log && p.min > 0.0 {
                                sl = sl.logarithmic(true);
                            }
                            ui.add(sl)
                        };
                        if (x - v).abs() > f32::EPSILON && instrument {
                            let _ = app.engine.execute_merged(
                                "mix.instrument_param",
                                &json!({"track": tid.0, "param": p.id, "value": x}),
                                &format!("instrument_param:{}:{}", tid.0, p.id),
                            );
                        } else if (x - v).abs() > f32::EPSILON {
                            let _ = app.engine.execute_merged(
                                "mix.insert_param",
                                &json!({"track": tid.0, "slot": slot, "param": p.id, "value": x}),
                                &format!("param:{}:{slot}:{}", tid.0, p.id),
                            );
                            app.touch_param(tid, slot, p.id, x);
                            if let Some(pl) = &app.player {
                                pl.editor_set_param(tid, slot, p.id, x);
                            }
                        }
                        let _ = resp;
                        let hosted = if instrument || !openpt_mix::is_third_party(&ins.plugin) {
                            None
                        } else {
                            app.player.as_ref().and_then(|pl| pl.editor_value_text(tid, slot, p.id, x))
                        };
                        let shown = match hosted {
                            Some(text) => text,
                            None if instrument => crate::sampler_window::format_param(&ins.plugin, p, x),
                            None => p.format(x),
                        };
                        ui.label(egui::RichText::new(shown).font(mono(11.0)));
                        ui.end_row();
                    }
                });
            });
        });
        if let Some(w) = shown
            && instrument
            && ins.plugin == crate::sampler_window::SAMPLER
        {
            crate::sampler_window::drop_target(app, ctx, w.response.rect, tid);
        }
        if open {
            keep.push((tid, slot));
        }
    }
    app.ui.plugin_windows = keep;
}

fn eq_curve(ui: &mut Ui, id: &str, ins: &openpt_model::Insert) {
    let t = Tokens::current();
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().max(300.0), 110.0), Sense::hover());
    ui.painter().rect_filled(r, 4.0, Color32::from_rgb(14, 18, 22));
    let n = 160;
    let freqs: Vec<f32> = (0..n).map(|i| 20.0 * 1000f32.powf(i as f32 / (n - 1) as f32)).collect();
    let params: Vec<(&str, f32)> = ins.params.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let resp =
        if id == "eq_7band" { openpt_dsp::eq7_response(&params, &freqs, 48_000.0) } else { openpt_dsp::eq1_response(&params, &freqs, 48_000.0) };
    for db in [-12.0f32, 0.0, 12.0] {
        let y = r.center().y - db / 18.0 * r.height() * 0.5;
        ui.painter().line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], Stroke::new(1.0, Color32::from_rgb(40, 50, 60)));
    }
    let pts: Vec<egui::Pos2> = resp
        .iter()
        .enumerate()
        .map(|(i, db)| pos2(r.min.x + r.width() * i as f32 / (n - 1) as f32, r.center().y - db.clamp(-18.0, 18.0) / 18.0 * r.height() * 0.5))
        .collect();
    ui.painter().add(egui::Shape::line(pts, Stroke::new(2.0, t.counter_text)));
}

fn about(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_about;
    if !open {
        return;
    }
    let tab_id = egui::Id::new("about_tab");
    egui::Window::new("About OpenPT").open(&mut open).default_size(vec2(640.0, 420.0)).collapsible(false).show(ctx, |ui| {
        let mut tab = ui.data_mut(|d| d.get_temp::<u8>(tab_id)).unwrap_or(0);
        ui.horizontal(|ui| {
            for (i, l) in (0u8..).zip(["About", "Licenses"]) {
                if ui.selectable_label(tab == i, l).clicked() {
                    tab = i;
                }
            }
        });
        ui.data_mut(|d| d.insert_temp(tab_id, tab));
        ui.separator();
        match tab {
            1 => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for text in [include_str!("../../../NOTICE"), include_str!("../../../LICENSE-MIT"), include_str!("../../../LICENSE-APACHE")] {
                        ui.label(egui::RichText::new(text).font(mono(11.0)));
                        ui.separator();
                    }
                });
            }
            _ => {
                ui.label(egui::RichText::new("OpenPT").font(bold(22.0)));
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("Digital audio workstation. Made with Rust and egui.");
            }
        }
    });
    app.ui.show_about = open;
}

fn session_info(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_session_info;
    if !open {
        return;
    }
    egui::Window::new("Session Info").open(&mut open).default_size(vec2(480.0, 360.0)).show(ctx, |ui| {
        let text = openpt_engine::inspect::session_text(&app.engine);
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(egui::RichText::new(text).font(mono(11.0)));
        });
    });
    app.ui.show_session_info = open;
}

fn preset_dir(app: &SoundApp, plugin: &str) -> Option<std::path::PathBuf> {
    app.preset_dir.as_ref().map(|d| d.join(crate::preset_folder_name(plugin)))
}

pub(crate) fn preset_names(app: &SoundApp, plugin: &str) -> Vec<String> {
    let Some(dir) = preset_dir(app, plugin) else { return Vec::new() };
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().filter_map(|e| e.path().file_stem().and_then(|x| x.to_str()).map(str::to_string)).collect())
        .unwrap_or_default();
    names.sort();
    names
}

pub(crate) fn recall_preset(app: &mut SoundApp, tid: openpt_model::TrackId, slot: usize, info: &openpt_dsp::PluginInfo, name: Option<&str>) {
    let Some(name) = name else {
        let defaults: serde_json::Map<String, serde_json::Value> = info.params.iter().map(|p| (p.id.to_string(), json!(p.default))).collect();
        let _ = app.run("mix.insert_params", json!({"track": tid.0, "slot": slot, "params": defaults, "preset": "<factory default>"}));
        return;
    };
    let Some(dir) = preset_dir(app, info.id) else { return };
    let Some(v) = std::fs::read_to_string(dir.join(format!("{name}.json"))).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
    else {
        app.ui.status = format!("Could not read the preset {name}");
        return;
    };
    if let Some(state) = v.get("state").and_then(serde_json::Value::as_str)
        && let Some(bytes) = openpt_model::b64::decode(state)
        && let Some(p) = &app.player
    {
        if !p.load_plugin_state(tid, slot, &bytes) {
            app.ui.status = format!("{} refused the preset {name}", info.name);
            return;
        }
        let ids: Vec<String> = app
            .engine
            .session()
            .track(tid)
            .and_then(|t| t.mixer.inserts.get(slot).cloned().flatten())
            .map(|i| i.params.keys().cloned().collect())
            .unwrap_or_default();
        let params: serde_json::Map<String, serde_json::Value> = p.editor_params(tid, slot, &ids).into_iter().map(|(k, x)| (k, json!(x))).collect();
        let _ = app.engine.execute("mix.insert_state", &json!({"track": tid.0, "slot": slot, "state": state}));
        let _ = app.run("mix.insert_params", json!({"track": tid.0, "slot": slot, "params": params, "preset": name}));
        return;
    }
    let params: serde_json::Map<String, serde_json::Value> = v
        .get("params")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(k, val)| info.params.iter().any(|pi| pi.id == k.as_str()) && val.as_f64().is_some_and(f64::is_finite))
        .map(|(k, val)| (k.clone(), val.clone()))
        .collect();
    let _ = app.run("mix.insert_params", json!({"track": tid.0, "slot": slot, "params": params, "preset": name}));
}

pub(crate) fn save_preset(
    app: &mut SoundApp,
    tid: openpt_model::TrackId,
    slot: usize,
    info: &openpt_dsp::PluginInfo,
    ins: &openpt_model::Insert,
    name: &str,
) {
    let Some(dir) = preset_dir(app, info.id) else { return };
    let clean = openpt_engine::io::sanitize_name(name);
    let mut body = json!({"format": "openpt-plugin-preset", "plugin": info.id, "params": ins.params});
    if openpt_mix::is_third_party(&ins.plugin)
        && let Some(state) = app.player.as_ref().and_then(|p| p.plugin_state(tid, slot))
    {
        body["state"] = json!(openpt_model::b64::encode(&state));
    }
    let ok = std::fs::create_dir_all(&dir).is_ok() && std::fs::write(dir.join(format!("{clean}.json")), body.to_string()).is_ok();
    app.ui.status = if ok { format!("Saved preset {clean}") } else { "Could not save the preset".into() };
    if ok {
        let _ = app.run("mix.insert_params", json!({"track": tid.0, "slot": slot, "params": {}, "preset": clean}));
    }
}

fn preset_menu(app: &mut SoundApp, ui: &mut Ui, tid: openpt_model::TrackId, slot: usize, info: &openpt_dsp::PluginInfo, ins: &openpt_model::Insert) {
    if ui.button("<factory default>").clicked() {
        recall_preset(app, tid, slot, info, None);
        ui.close();
    }
    for (name, values) in openpt_dsp::factory_presets(info.id) {
        if ui.button(name).clicked() {
            let mut params: serde_json::Map<String, serde_json::Value> = info.params.iter().map(|p| (p.id.to_string(), json!(p.default))).collect();
            for (k, v) in values {
                params.insert(k.to_string(), json!(v));
            }
            let _ = app.run("mix.insert_params", json!({"track": tid.0, "slot": slot, "params": params, "preset": name}));
            ui.close();
        }
    }
    if preset_dir(app, info.id).is_none() {
        ui.label("User presets need the desktop app.");
        return;
    }
    for n in preset_names(app, info.id) {
        if ui.button(&n).clicked() {
            recall_preset(app, tid, slot, info, Some(&n));
            ui.close();
        }
    }
    ui.separator();
    let key = egui::Id::new(("preset_name", tid.0, slot));
    let mut name: String = ui.ctx().memory(|m| m.data.get_temp(key)).unwrap_or_else(|| format!("{} preset", info.name));
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0));
        if ui.button("Save").clicked() {
            save_preset(app, tid, slot, info, ins, &name);
        }
    });
    ui.ctx().memory_mut(|m| m.data.insert_temp(key, name));
}
