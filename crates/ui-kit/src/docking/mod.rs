mod model;
mod surface;
mod targets;

pub use model::{Action, DockError, Floating, Layout, Location, MAX_DEPTH, MAX_PANELS, Node, Permissions, Placement, StackEntry, Zone};
pub use surface::{DockArea, DockContent, DockOutput, DockStyle, PanelLimits, begin_drag, cancel_drag};
pub use targets::{DropProposal, DropTarget, drop_zone, resolve_drop, zone_rect};

#[cfg(test)]
mod tests;
