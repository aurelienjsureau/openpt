use std::collections::HashMap;
use std::sync::Arc;

use egui::{ImageSource, load::Bytes};

#[derive(Clone, Copy, Debug)]
pub struct SvgStyle {
    pub current_color: &'static str,
    pub stroke_width: Option<&'static str>,
}

pub struct SvgIconSet {
    icons: HashMap<&'static str, Arc<[u8]>>,
    uri_prefix: &'static str,
    fallback: &'static str,
}

impl SvgIconSet {
    pub fn new(icons: &'static [(&'static str, &'static [u8])], style: SvgStyle, uri_prefix: &'static str, fallback: &'static str) -> Self {
        let stroke = style.stroke_width.map(|width| format!("stroke-width=\"{width}\""));
        let icons = icons
            .iter()
            .map(|(name, bytes)| {
                let mut svg = String::from_utf8_lossy(bytes).replace("currentColor", style.current_color);
                if let Some(stroke) = &stroke {
                    svg = svg.replace("stroke-width=\"2\"", stroke);
                }
                (*name, Arc::from(svg.into_bytes()))
            })
            .collect();
        Self { icons, uri_prefix, fallback }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.icons.contains_key(name)
    }

    pub fn canonical_name(&self, name: &str) -> Option<&'static str> {
        self.icons.get_key_value(name).map(|(key, _)| *key)
    }

    pub fn source(&self, name: &str) -> ImageSource<'static> {
        let bytes = self.icons.get(name).or_else(|| self.icons.get(self.fallback)).cloned().unwrap_or_default();
        ImageSource::Bytes { uri: format!("{}{name}.svg", self.uri_prefix).into(), bytes: Bytes::Shared(bytes) }
    }
}

#[cfg(test)]
mod tests;
