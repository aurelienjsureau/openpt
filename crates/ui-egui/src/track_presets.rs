use std::path::{Path, PathBuf};

pub const DEFAULT_CATEGORY: &str = "Default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackPreset {
    pub category: String,
    pub name: String,
    pub path: PathBuf,
}

pub fn dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("OPENPT_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d).join("Track Presets"));
    }
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    if let Some(a) = env("APPDATA") {
        return Some(a.join("OpenPT").join("Track Presets"));
    }
    env("HOME").map(|h| h.join(if cfg!(target_os = "macos") { "Library/Application Support/OpenPT/Track Presets" } else { ".config/OpenPT/Track Presets" }))
}

fn is_preset(p: &Path) -> bool {
    p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("json"))
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn list() -> Vec<TrackPreset> {
    let Some(root) = dir() else { return Vec::new() };
    let mut out = Vec::new();
    for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            let category = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            for f in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                let f = f.path();
                if is_preset(&f) {
                    out.push(TrackPreset { category: category.clone(), name: stem(&f), path: f });
                }
            }
        } else if is_preset(&p) {
            out.push(TrackPreset { category: DEFAULT_CATEGORY.into(), name: stem(&p), path: p });
        }
    }
    out.sort_by_key(|a| (a.category.to_lowercase(), a.name.to_lowercase()));
    out
}

pub fn categories() -> Vec<String> {
    let mut c: Vec<String> = list().into_iter().map(|p| p.category).collect();
    c.push(DEFAULT_CATEGORY.into());
    c.sort_by_key(|x| x.to_lowercase());
    c.dedup();
    c
}

fn clean(s: &str) -> String {
    s.trim().chars().map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { '_' } else { c }).collect()
}

pub fn path_for(category: &str, name: &str) -> Option<PathBuf> {
    let (category, name) = (clean(category), clean(name));
    if name.is_empty() {
        return None;
    }
    let category = if category.is_empty() { DEFAULT_CATEGORY.to_string() } else { category };
    Some(dir()?.join(category).join(format!("{name}.json")))
}

pub fn recall_menu(ui: &mut egui::Ui, on_pick: &mut dyn FnMut(&TrackPreset)) {
    let presets = list();
    if presets.is_empty() {
        ui.weak("No track presets yet (Track > Save Track Preset...)");
        return;
    }
    let mut cats: Vec<&str> = presets.iter().map(|p| p.category.as_str()).collect();
    cats.dedup();
    for cat in cats {
        ui.menu_button(cat, |ui| {
            for p in presets.iter().filter(|p| p.category == cat) {
                if ui.button(&p.name).clicked() {
                    on_pick(p);
                    ui.close();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_safe_for_the_file_system() {
        assert_eq!(clean(" Lead: Vox/Main? "), "Lead_ Vox_Main_");
        assert_eq!(clean("   "), "");
    }
}
