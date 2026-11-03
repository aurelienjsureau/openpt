use crate::SoundApp;
use crate::theme::{Tokens, bold};

pub(crate) fn plugin_name(id: &str) -> String {
    let bare = id.strip_prefix(openpt_model::MULTI_OUT_PREFIX).unwrap_or(id);
    let bare = bare.strip_prefix(openpt_model::MULTI_MONO_PREFIX).unwrap_or(bare);
    crate::mix_window::plugin_info(bare).map_or_else(|| bare.to_string(), |p| p.name.to_string())
}

pub fn plugin_load(app: &SoundApp, ctx: &egui::Context) {
    let Some(load) = &app.plugin_load else { return };
    let (done, total) = (load.done, load.total.max(1));
    let next = load.next.as_deref().map(plugin_name).unwrap_or_default();
    let session = app.engine.session().name.clone();
    egui::Modal::new(egui::Id::new("plugin_load_progress")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(egui::RichText::new(format!("Opening {session}")).font(bold(13.0)));
        ui.label(format!("Loading plug-ins: {done} of {total}"));
        ui.add(egui::ProgressBar::new(done as f32 / total as f32).show_percentage());
        ui.label(egui::RichText::new(next).small().color(Tokens::current().text_dim));
    });
    ctx.request_repaint();
}
