use egui::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SplitSize {
    Ratio(f32),
    FixedFirst(f32),
    FixedSecond(f32),
}

impl SplitSize {
    fn value(self) -> f32 {
        match self {
            Self::Ratio(v) | Self::FixedFirst(v) | Self::FixedSecond(v) => v,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitLayout {
    pub first: Rect,
    pub divider: Rect,
    pub second: Rect,
    first_size: f32,
    available: f32,
}

impl SplitLayout {
    pub fn first_size(self) -> f32 {
        self.first_size
    }

    pub fn available(self) -> f32 {
        self.available
    }

    pub fn resized(self, size: SplitSize, delta: f32, minimum: f32) -> Option<SplitSize> {
        if !size.value().is_finite() || !delta.is_finite() || !valid_nonnegative(minimum) || self.available <= 0.0 {
            return None;
        }
        let available = f64::from(self.available);
        let first = clamp_first(f64::from(self.first_size) + f64::from(delta), available, minimum);
        Some(match size {
            SplitSize::Ratio(_) => SplitSize::Ratio((first / available) as f32),
            SplitSize::FixedFirst(_) => SplitSize::FixedFirst(first as f32),
            SplitSize::FixedSecond(_) => SplitSize::FixedSecond((available - first) as f32),
        })
    }
}

pub fn split_rect(rect: Rect, axis: SplitAxis, size: SplitSize, gap: f32, minimum: f32) -> Option<SplitLayout> {
    if !valid_rect(rect) || !valid_nonnegative(gap) || !valid_nonnegative(minimum) || !size.value().is_finite() {
        return None;
    }
    let total = match axis {
        SplitAxis::Horizontal => rect.width(),
        SplitAxis::Vertical => rect.height(),
    };
    let gap = gap.min(total);
    let available = total - gap;
    let space = f64::from(available);
    let wanted = match size {
        SplitSize::Ratio(ratio) => space * f64::from(ratio),
        SplitSize::FixedFirst(points) => f64::from(points),
        SplitSize::FixedSecond(points) => space - f64::from(points),
    };
    let first_size = clamp_first(wanted, space, minimum) as f32;
    let mut first = rect;
    let mut divider = rect;
    let mut second = rect;
    match axis {
        SplitAxis::Horizontal => {
            first.max.x = (rect.min.x + first_size).min(rect.max.x);
            divider.min.x = first.max.x;
            divider.max.x = (divider.min.x + gap).min(rect.max.x);
            second.min.x = divider.max.x;
        }
        SplitAxis::Vertical => {
            first.max.y = (rect.min.y + first_size).min(rect.max.y);
            divider.min.y = first.max.y;
            divider.max.y = (divider.min.y + gap).min(rect.max.y);
            second.min.y = divider.max.y;
        }
    }
    Some(SplitLayout { first, divider, second, first_size, available })
}

pub(super) fn valid_rect(rect: Rect) -> bool {
    rect.min.is_finite()
        && rect.max.is_finite()
        && rect.width().is_finite()
        && rect.height().is_finite()
        && rect.min.x <= rect.max.x
        && rect.min.y <= rect.max.y
}

fn valid_nonnegative(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}

fn clamp_first(wanted: f64, available: f64, minimum: f32) -> f64 {
    let low = f64::from(minimum).min(available * 0.5);
    wanted.clamp(low, available - low)
}
