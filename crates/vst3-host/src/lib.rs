#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod scan;

#[cfg(not(target_arch = "wasm32"))]
mod isolated;
#[cfg(not(target_arch = "wasm32"))]
pub use isolated::{Scanner, scan_child, set_scanner};

#[cfg(all(not(target_arch = "wasm32"), target_arch = "x86_64"))]
#[allow(unsafe_code)]
mod ara;
#[cfg(all(not(target_arch = "wasm32"), target_arch = "x86_64"))]
#[allow(unsafe_code)]
mod ara_sys;
#[cfg(not(target_arch = "wasm32"))]
#[allow(unsafe_code)]
mod ffi;
#[cfg(all(not(target_arch = "wasm32"), target_arch = "x86_64"))]
pub use ara::{
    PlaybackRequest, bindings as ara_bindings, bound_tracks as ara_bound_tracks, finish_restores as ara_finish_restores,
    set_pending_archives as ara_set_pending_archives, store_documents as ara_store_documents, sync_track as ara_sync_track,
    take_playback_requests as ara_playback_requests,
};
#[cfg(not(target_arch = "wasm32"))]
mod plugin;
#[cfg(not(target_arch = "wasm32"))]
mod registry;

#[cfg(not(target_arch = "wasm32"))]
pub use plugin::Vst3Plugin;

use openpt_dsp::{Category, Plugin, PluginInfo};
use std::path::{Path, PathBuf};

static UI_WAKER: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

pub fn set_ui_waker(f: impl Fn() + Send + Sync + 'static) {
    let _ = UI_WAKER.set(Box::new(f));
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn wake_ui() {
    if let Some(f) = UI_WAKER.get() {
        f();
    }
}

pub const ID_PREFIX: &str = "vst3:";

pub const AUDIO_MODULE_CLASS: &str = "Audio Module Class";

pub fn parse_id(id: &str) -> Option<[u8; 16]> {
    let hex = id.strip_prefix(ID_PREFIX)?;
    if hex.len() != 32 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        let pair = hex.get(i * 2..i * 2 + 2)?;
        *o = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Vst3Descriptor {
    pub id: String,
    pub class_id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub sdk_version: String,
    pub sub_categories: Vec<String>,
    pub category: Category,
    pub is_instrument: bool,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Vst3Error {
    #[error("cannot load VST3 plugin {0}: {1}")]
    Load(String, String),
    #[error("no VST3 plugin `{0}`")]
    NotFound(String),
    #[error("cannot create VST3 plugin `{0}`: {1}")]
    Instantiate(String, String),
    #[error("VST3 processing: {0}")]
    Process(String),
    #[error("VST3 plugins are not supported on this platform")]
    Unsupported,
}

#[cfg(not(target_arch = "wasm32"))]
mod api {
    use super::*;

    pub fn cached() -> Option<Vec<Vst3Descriptor>> {
        registry::cached()
    }
    pub fn scan_in_background() -> bool {
        registry::scan_in_background()
    }
    pub fn scan_progress() -> (usize, usize) {
        crate::isolated::progress()
    }
    pub fn scan() -> Vec<Vst3Descriptor> {
        registry::scan()
    }
    pub fn rescan() -> Vec<Vst3Descriptor> {
        registry::rescan()
    }
    pub fn add_search_dir(dir: &Path) -> usize {
        registry::add_search_dir(dir)
    }
    pub fn scan_paths(dirs: &[PathBuf]) -> Vec<Vst3Descriptor> {
        registry::scan_paths(dirs)
    }
    pub fn load_bundle(path: &Path) -> Result<Vec<Vst3Descriptor>, Vst3Error> {
        registry::load_bundle(path)
    }
    pub fn instantiate(id: &str) -> Result<Box<dyn Plugin>, Vst3Error> {
        registry::instantiate(id)
    }
    pub fn plugin_info(id: &str) -> Option<&'static PluginInfo> {
        registry::plugin_info(id)
    }
    pub fn shutdown() {
        ffi::exit_modules();
    }
}

#[cfg(target_arch = "wasm32")]
mod api {
    use super::*;

    pub fn cached() -> Option<Vec<Vst3Descriptor>> {
        Some(Vec::new())
    }
    pub fn scan_in_background() -> bool {
        false
    }
    pub fn scan_progress() -> (usize, usize) {
        (0, 0)
    }
    pub fn scan() -> Vec<Vst3Descriptor> {
        Vec::new()
    }
    pub fn rescan() -> Vec<Vst3Descriptor> {
        Vec::new()
    }
    pub fn add_search_dir(_dir: &Path) -> usize {
        0
    }
    pub fn scan_paths(_dirs: &[PathBuf]) -> Vec<Vst3Descriptor> {
        Vec::new()
    }
    pub fn load_bundle(_path: &Path) -> Result<Vec<Vst3Descriptor>, Vst3Error> {
        Err(Vst3Error::Unsupported)
    }
    pub fn instantiate(_id: &str) -> Result<Box<dyn Plugin>, Vst3Error> {
        Err(Vst3Error::Unsupported)
    }
    pub fn plugin_info(_id: &str) -> Option<&'static PluginInfo> {
        None
    }
    pub fn shutdown() {}
}

pub fn scan() -> Vec<Vst3Descriptor> {
    api::scan()
}

pub fn rescan() -> Vec<Vst3Descriptor> {
    api::rescan()
}

pub fn add_search_dir(dir: &Path) -> usize {
    api::add_search_dir(dir)
}

pub fn scan_paths(dirs: &[PathBuf]) -> Vec<Vst3Descriptor> {
    api::scan_paths(dirs)
}

pub fn cached() -> Option<Vec<Vst3Descriptor>> {
    api::cached()
}

pub fn scan_in_background() -> bool {
    api::scan_in_background()
}

pub fn scan_progress() -> (usize, usize) {
    api::scan_progress()
}

pub fn shutdown() {
    api::shutdown()
}

pub fn load_bundle(path: &Path) -> Result<Vec<Vst3Descriptor>, Vst3Error> {
    api::load_bundle(path)
}

pub fn instantiate(id: &str) -> Result<Box<dyn Plugin>, Vst3Error> {
    api::instantiate(id)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn instantiate_plugin(id: &str) -> Result<Vst3Plugin, Vst3Error> {
    registry::instantiate_plugin(id)
}

pub fn create(id: &str) -> Option<Box<dyn Plugin>> {
    parse_id(id)?;
    match api::instantiate(id) {
        Ok(p) => Some(p),
        Err(e) => {
            log::warn!("{e}");
            None
        }
    }
}

pub fn plugin_info(id: &str) -> Option<&'static PluginInfo> {
    api::plugin_info(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_parse() {
        let id = "vst3:0123456789ABCDEFfedcba9876543210";
        assert_eq!(parse_id(id), Some([0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x10]));
        assert_eq!(parse_id("vst3:"), None);
        assert_eq!(parse_id("vst3:0123"), None);
        assert_eq!(parse_id("vst3:0123456789ABCDEF0123456789ABCDEG"), None);
        assert_eq!(parse_id("vst3:ééééééééééééééééé"), None);
        assert_eq!(parse_id("VST3:0123456789ABCDEF0123456789ABCDEF"), None);
        assert_eq!(parse_id("au:x"), None);
        assert_eq!(parse_id("eq_7band"), None);
    }

    #[test]
    fn non_vst3_ids_are_rejected_without_scanning() {
        assert!(create("eq_7band").is_none());
        assert!(create("au:com.x").is_none());
        assert!(plugin_info("reverb").is_none());
    }

    #[test]
    fn garbage_files_are_errors_not_crashes() {
        let d = std::env::temp_dir().join(format!("openpt-vst3-garbage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let arch = if cfg!(target_os = "macos") {
            "MacOS"
        } else if cfg!(windows) {
            "x86_64-win"
        } else {
            "x86_64-linux"
        };
        let exe = if cfg!(target_os = "macos") {
            "Bundle"
        } else if cfg!(windows) {
            "Bundle.vst3"
        } else {
            "Bundle.so"
        };
        std::fs::create_dir_all(d.join(format!("Bundle.vst3/Contents/{arch}"))).unwrap();
        std::fs::write(d.join("garbage.vst3"), b"\x7fELF this is not a plugin \x00\x01\x02").unwrap();
        std::fs::write(d.join("empty.vst3"), b"").unwrap();
        std::fs::write(d.join(format!("Bundle.vst3/Contents/{arch}/{exe}")), [0xcf, 0xfa, 0xed, 0xfe, 1, 2, 3]).unwrap();
        std::fs::create_dir_all(d.join("NoBinary.vst3/Contents")).unwrap();
        for f in ["garbage.vst3", "empty.vst3", "Bundle.vst3", "NoBinary.vst3", "missing.vst3"] {
            assert!(load_bundle(&d.join(f)).is_err(), "{f}");
        }
        assert!(scan_paths(std::slice::from_ref(&d)).is_empty());
        let missing = "vst3:00000000000000000000000000000000";
        assert!(create(missing).is_none());
        assert!(plugin_info(missing).is_none());
        assert!(instantiate(missing).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_real_library_without_the_vst3_export_is_rejected() {
        let lib = if cfg!(target_os = "macos") {
            None
        } else {
            ["/lib/x86_64-linux-gnu/libz.so.1", "/usr/lib/libz.so.1", "/lib64/libz.so.1"].iter().map(PathBuf::from).find(|p| p.exists())
        };
        let Some(lib) = lib else { return };
        let d = std::env::temp_dir().join(format!("openpt-vst3-nolib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::os::unix::fs::symlink(&lib, d.join("z.vst3")).unwrap();
        let e = load_bundle(&d.join("z.vst3")).unwrap_err();
        assert!(e.to_string().contains("not a VST3 plugin"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
