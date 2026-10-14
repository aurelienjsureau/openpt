mod geometry;
mod resize;
mod toolbar;

pub use geometry::{SplitAxis, SplitLayout, SplitSize, split_rect};
pub use resize::{ResizeHandle, ResizeHandleStyle, ResizeResponse};
pub use toolbar::{ToolbarFit, fit_toolbar};

#[cfg(test)]
mod tests;
