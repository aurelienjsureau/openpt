use egui::{Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonVisuals {
    pub fill: Color32,
    pub stroke: Stroke,
    pub icon: Color32,
}

#[derive(Clone, Copy, Debug)]
pub struct IconButtonStyle {
    pub normal: ButtonVisuals,
    pub hovered: ButtonVisuals,
    pub pressed: Option<ButtonVisuals>,
    pub selected: ButtonVisuals,
    pub corner_radius: CornerRadius,
    pub focus_stroke: Stroke,
}

impl IconButtonStyle {
    fn visuals(&self, selected: bool, hovered: bool, pressed: bool) -> ButtonVisuals {
        if selected {
            self.selected
        } else if let Some(visuals) = self.pressed.filter(|_| pressed) {
            visuals
        } else if hovered {
            self.hovered
        } else {
            self.normal
        }
    }
}

pub struct IconButton<'a> {
    label: &'a str,
    size: Vec2,
    selected: bool,
    style: &'a IconButtonStyle,
    single_shape_frame: bool,
}

impl<'a> IconButton<'a> {
    pub fn new(label: &'a str, size: Vec2, style: &'a IconButtonStyle) -> Self {
        Self { label, size, selected: false, style, single_shape_frame: false }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn single_shape_frame(mut self) -> Self {
        self.single_shape_frame = true;
        self
    }

    pub fn show(self, ui: &mut Ui, paint_icon: impl FnOnce(&Ui, Rect, Color32)) -> Response {
        let valid = self.size.is_finite() && self.size.x > 0.0 && self.size.y > 0.0 && self.size.x <= 4096.0 && self.size.y <= 4096.0;
        let (rect, response) =
            ui.allocate_exact_size(if valid { self.size } else { Vec2::ZERO }, if valid { Sense::click() } else { Sense::hover() });
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, valid && ui.is_enabled(), self.selected, self.label));
        if !valid {
            return response;
        }

        let visuals = self.style.visuals(self.selected, response.hovered(), response.is_pointer_button_down_on());
        let stroke = bounded_stroke(visuals.stroke, rect);
        if self.single_shape_frame {
            ui.painter().rect(rect, self.style.corner_radius, visuals.fill, stroke, StrokeKind::Inside);
        } else {
            if visuals.fill != Color32::TRANSPARENT {
                ui.painter().rect_filled(rect, self.style.corner_radius, visuals.fill);
            }
            if !stroke.is_empty() {
                ui.painter().rect_stroke(rect, self.style.corner_radius, stroke, StrokeKind::Inside);
            }
        }
        paint_icon(ui, rect, visuals.icon);
        if response.has_focus() && rect.width() > 4.0 && rect.height() > 4.0 {
            let focus_rect = rect.shrink(2.0);
            let stroke = bounded_stroke(self.style.focus_stroke, focus_rect);
            if !stroke.is_empty() {
                ui.painter().rect_stroke(focus_rect, self.style.corner_radius, stroke, StrokeKind::Inside);
            }
        }
        response
    }
}

fn bounded_stroke(stroke: Stroke, rect: Rect) -> Stroke {
    if stroke.width.is_finite() && stroke.width > 0.0 {
        Stroke::new(stroke.width.min(rect.width().min(rect.height()) * 0.5), stroke.color)
    } else {
        Stroke::NONE
    }
}

#[cfg(test)]
mod tests;
