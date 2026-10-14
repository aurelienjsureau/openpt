use std::ops::RangeInclusive;

use egui::{Color32, EventFilter, Id, Key, Modifiers, Rect, Response, Sense, Ui, Vec2, accesskit};

use super::SplitAxis;
use super::geometry::valid_rect;

#[derive(Clone, Copy, Debug)]
pub struct ResizeHandleStyle {
    pub hovered: Color32,
    pub dragged: Color32,
    pub focused: Color32,
}

pub struct ResizeResponse {
    pub response: Response,
    pub delta: f32,
}

pub struct ResizeHandle<'a> {
    id: Id,
    axis: SplitAxis,
    rect: Rect,
    label: &'a str,
    style: ResizeHandleStyle,
    hit_margin: f32,
    step: f32,
    value: Option<(f32, RangeInclusive<f32>)>,
}

impl<'a> ResizeHandle<'a> {
    pub fn new(id: Id, axis: SplitAxis, rect: Rect, label: &'a str, style: ResizeHandleStyle) -> Self {
        Self { id, axis, rect, label, style, hit_margin: 3.0, step: 8.0, value: None }
    }

    pub fn hit_margin(mut self, margin: f32) -> Self {
        self.hit_margin = margin;
        self
    }

    pub fn keyboard_step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    pub fn value(mut self, value: f32, range: RangeInclusive<f32>) -> Self {
        self.value = Some((value, range));
        self
    }

    pub fn show(self, ui: &mut Ui) -> ResizeResponse {
        let hit = self.rect.expand2(match self.axis {
            SplitAxis::Horizontal => Vec2::new(self.hit_margin, 0.0),
            SplitAxis::Vertical => Vec2::new(0.0, self.hit_margin),
        });
        let valid = valid_rect(self.rect)
            && self.rect.width() > 0.0
            && self.rect.height() > 0.0
            && valid_rect(hit)
            && self.hit_margin.is_finite()
            && (0.0..=4096.0).contains(&self.hit_margin)
            && self.step.is_finite()
            && self.step > 0.0
            && self.value.as_ref().is_none_or(|(value, range)| {
                value.is_finite() && range.start().is_finite() && range.end().is_finite() && range.start() <= range.end()
            });
        let inert = Rect::from_min_size(ui.min_rect().min, Vec2::ZERO);
        let mut response = ui.interact(if valid { hit } else { inert }, self.id, if valid { Sense::drag() } else { Sense::hover() });
        let enabled = valid && response.enabled();
        let mut delta = 0.0;
        if enabled {
            if response.drag_started() {
                response.request_focus();
            }
            if response.dragged() {
                let drag = response.drag_delta();
                delta = match self.axis {
                    SplitAxis::Horizontal => drag.x,
                    SplitAxis::Vertical => drag.y,
                };
            }
            if response.has_focus() {
                ui.memory_mut(|memory| {
                    memory.set_focus_lock_filter(
                        response.id,
                        EventFilter {
                            horizontal_arrows: self.axis == SplitAxis::Horizontal,
                            vertical_arrows: self.axis == SplitAxis::Vertical,
                            ..Default::default()
                        },
                    );
                });
                let (back, forward) = match self.axis {
                    SplitAxis::Horizontal => (Key::ArrowLeft, Key::ArrowRight),
                    SplitAxis::Vertical => (Key::ArrowUp, Key::ArrowDown),
                };
                ui.input_mut(|input| {
                    if input.consume_key(Modifiers::NONE, back) {
                        delta -= self.step;
                    }
                    if input.consume_key(Modifiers::NONE, forward) {
                        delta += self.step;
                    }
                });
            }
            ui.input_mut(|input| {
                input.consume_accesskit_action_requests(response.id, |request| match request.action {
                    accesskit::Action::Increment => {
                        delta += self.step;
                        true
                    }
                    accesskit::Action::Decrement => {
                        delta -= self.step;
                        true
                    }
                    accesskit::Action::SetValue => {
                        if let (Some((value, range)), Some(accesskit::ActionData::NumericValue(new_value))) = (&self.value, &request.data)
                            && new_value.is_finite()
                        {
                            delta = (new_value.clamp(f64::from(*range.start()), f64::from(*range.end())) - f64::from(*value)) as f32;
                        }
                        true
                    }
                    _ => false,
                });
            });
        }
        if !delta.is_finite() {
            delta = 0.0;
        }
        if delta != 0.0 {
            response.mark_changed();
        }
        response.widget_info(|| {
            if let Some((value, _)) = &self.value
                && valid
            {
                egui::WidgetInfo::slider(enabled, f64::from(*value), self.label)
            } else {
                egui::WidgetInfo::labeled(egui::WidgetType::Other, enabled, self.label)
            }
        });
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_role(accesskit::Role::Splitter);
            node.set_orientation(match self.axis {
                SplitAxis::Horizontal => accesskit::Orientation::Vertical,
                SplitAxis::Vertical => accesskit::Orientation::Horizontal,
            });
            if enabled {
                node.add_action(accesskit::Action::Increment);
                node.add_action(accesskit::Action::Decrement);
                if let Some((value, range)) = &self.value {
                    node.set_numeric_value(f64::from(*value));
                    node.set_min_numeric_value(f64::from(*range.start()));
                    node.set_max_numeric_value(f64::from(*range.end()));
                    node.set_numeric_value_step(f64::from(self.step));
                    node.add_action(accesskit::Action::SetValue);
                }
            }
        });
        if enabled {
            let fill = if response.dragged() {
                Some(self.style.dragged)
            } else if response.has_focus() {
                Some(self.style.focused)
            } else if response.hovered() {
                Some(self.style.hovered)
            } else {
                None
            };
            if let Some(fill) = fill {
                ui.painter().rect_filled(self.rect, 0.0, fill);
            }
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(match self.axis {
                    SplitAxis::Horizontal => egui::CursorIcon::ResizeHorizontal,
                    SplitAxis::Vertical => egui::CursorIcon::ResizeVertical,
                });
            }
        }
        ResizeResponse { response, delta }
    }
}
