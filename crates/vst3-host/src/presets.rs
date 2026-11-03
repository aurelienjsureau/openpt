use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactoryPreset {
    pub name: String,
    pub folder: String,
    pub path: PathBuf,
}

impl FactoryPreset {
    pub fn label(&self) -> String {
        if self.folder.is_empty() { self.name.clone() } else { format!("{} › {}", self.folder, self.name) }
    }
}

type Index = HashMap<String, Vec<FactoryPreset>>;

static INDEX: OnceLock<Index> = OnceLock::new();
static STARTED: Mutex<bool> = Mutex::new(false);

pub fn roots() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut out = Vec::new();
    if cfg!(windows) {
        out.extend(env("PROGRAMDATA").map(|d| d.join("VST3 Presets")));
        out.extend(env("USERPROFILE").map(|d| d.join("Documents").join("VST3 Presets")));
        out.extend(env("APPDATA").map(|d| d.join("VST3 Presets")));
    } else if cfg!(target_os = "macos") {
        out.push(PathBuf::from("/Library/Audio/Presets"));
        out.extend(env("HOME").map(|d| d.join("Library/Audio/Presets")));
    } else {
        out.push(PathBuf::from("/usr/share/vst3/presets"));
        out.push(PathBuf::from("/usr/local/share/vst3/presets"));
        out.extend(env("HOME").map(|d| d.join(".vst3/presets")));
    }
    out
}

pub fn index_in_background() {
    let mut started = STARTED.lock().unwrap_or_else(PoisonError::into_inner);
    if *started || INDEX.get().is_some() {
        return;
    }
    *started = true;
    let spawned = std::thread::Builder::new().name("openpt-presets".into()).spawn(|| {
        let t0 = std::time::Instant::now();
        let index = build(&roots());
        log::info!("factory presets: {} plug-ins, {} presets indexed in {} ms", index.len(), index.values().map(Vec::len).sum::<usize>(), t0.elapsed().as_millis());
        let _ = INDEX.set(index);
    });
    if let Err(e) = spawned {
        log::warn!("preset index thread failed: {e}");
    }
}

pub fn ready() -> bool {
    INDEX.get().is_some()
}

pub fn for_plugin(id: &str) -> Vec<FactoryPreset> {
    let Some(index) = INDEX.get() else { return Vec::new() };
    let hex = id.strip_prefix(crate::ID_PREFIX).unwrap_or(id).to_ascii_uppercase();
    let mut out = index.get(&hex).cloned().unwrap_or_default();
    if let Some(alt) = com_order(&hex)
        && alt != hex
        && let Some(more) = index.get(&alt)
    {
        out.extend(more.iter().cloned());
    }
    out.sort_by(|a, b| a.folder.to_lowercase().cmp(&b.folder.to_lowercase()).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

fn com_order(hex: &str) -> Option<String> {
    if hex.len() != 32 || !hex.is_ascii() {
        return None;
    }
    let b = |i: usize| &hex[i * 2..i * 2 + 2];
    let order = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
    Some(order.iter().map(|&i| b(i)).collect())
}

pub fn build(roots: &[PathBuf]) -> Index {
    let mut index = Index::new();
    for root in roots {
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let path = e.path();
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("vstpreset"))
                    && let Some(cid) = class_id(&path)
                {
                    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
                    let folder = folder_of(root, &path);
                    index.entry(cid).or_default().push(FactoryPreset { name, folder, path });
                }
            }
        }
    }
    index
}

fn folder_of(root: &Path, path: &Path) -> String {
    let Ok(rel) = path.strip_prefix(root) else { return String::new() };
    let dirs: Vec<String> = rel.parent().into_iter().flat_map(|p| p.components()).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    dirs.get(2..).map(|d| d.join(" › ")).unwrap_or_default()
}

fn class_id(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 40];
    f.read_exact(&mut head).ok()?;
    parse_class_id(&head)
}

fn parse_class_id(head: &[u8]) -> Option<String> {
    if head.get(..4)? != b"VST3" {
        return None;
    }
    let cid = std::str::from_utf8(head.get(8..40)?).ok()?;
    cid.bytes().all(|b| b.is_ascii_hexdigit()).then(|| cid.to_ascii_uppercase())
}

const MAX_PRESET_BYTES: usize = 64 << 20;

pub fn read_state(path: &Path) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok()?;
    (data.len() <= MAX_PRESET_BYTES).then_some(())?;
    state_from_vstpreset(&data)
}

pub fn state_from_vstpreset(data: &[u8]) -> Option<Vec<u8>> {
    parse_class_id(data)?;
    let list_at = usize::try_from(i64::from_le_bytes(data.get(40..48)?.try_into().ok()?)).ok()?;
    let list = data.get(list_at..)?;
    if list.get(..4)? != b"List" {
        return None;
    }
    let count = u32::from_le_bytes(list.get(4..8)?.try_into().ok()?) as usize;
    let mut comp: Option<&[u8]> = None;
    let mut ctrl: &[u8] = &[];
    for k in 0..count.min(64) {
        let e = list.get(8 + k * 20..8 + (k + 1) * 20)?;
        let off = usize::try_from(i64::from_le_bytes(e[4..12].try_into().ok()?)).ok()?;
        let size = usize::try_from(i64::from_le_bytes(e[12..20].try_into().ok()?)).ok()?;
        let body = data.get(off..off.checked_add(size)?)?;
        match &e[..4] {
            b"Comp" => comp = Some(body),
            b"Cont" => ctrl = body,
            _ => {}
        }
    }
    let comp = comp?;
    let mut out = Vec::with_capacity(12 + comp.len() + ctrl.len());
    out.extend_from_slice(crate::ffi::STATE_MAGIC);
    out.extend_from_slice(&u32::try_from(comp.len()).ok()?.to_le_bytes());
    out.extend_from_slice(comp);
    out.extend_from_slice(&u32::try_from(ctrl.len()).ok()?.to_le_bytes());
    out.extend_from_slice(ctrl);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(cid: &str, comp: &[u8], ctrl: &[u8]) -> Vec<u8> {
        let mut d = b"VST3".to_vec();
        d.extend_from_slice(&1i32.to_le_bytes());
        d.extend_from_slice(cid.as_bytes());
        let list_at = 48 + comp.len() + ctrl.len();
        d.extend_from_slice(&(list_at as i64).to_le_bytes());
        d.extend_from_slice(comp);
        d.extend_from_slice(ctrl);
        d.extend_from_slice(b"List");
        d.extend_from_slice(&2u32.to_le_bytes());
        for (id, off, len) in [(b"Comp", 48usize, comp.len()), (b"Cont", 48 + comp.len(), ctrl.len())] {
            d.extend_from_slice(id);
            d.extend_from_slice(&(off as i64).to_le_bytes());
            d.extend_from_slice(&(len as i64).to_le_bytes());
        }
        d
    }

    #[test]
    fn a_vstpreset_becomes_an_openpt_state_blob() {
        let d = preset("ABCDEF019182FAEB4272616250535631", b"component", b"ctl");
        let s = state_from_vstpreset(&d).unwrap();
        let (comp, ctrl) = crate::ffi::split_state(&s).unwrap();
        assert_eq!((comp, ctrl), (&b"component"[..], &b"ctl"[..]));
    }

    #[test]
    fn windows_preset_ids_match_raw_class_ids() {
        assert_eq!(com_order("01EFCDAB8291EBFA4272616250535631").as_deref(), Some("ABCDEF019182FAEB4272616250535631"));
    }

    #[test]
    fn presets_are_found_under_their_folders() {
        let root = std::env::temp_dir().join(format!("openpt-presets-{}", std::process::id()));
        let dir = root.join("Vendor").join("Plug").join("Bass");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Fat.vstpreset"), preset("ABCDEF019182FAEB4272616250535631", b"x", b"")).unwrap();
        std::fs::write(root.join("Vendor").join("Plug").join("Init.vstpreset"), preset("ABCDEF019182FAEB4272616250535631", b"y", b"")).unwrap();
        let index = build(std::slice::from_ref(&root));
        let got = index.get("ABCDEF019182FAEB4272616250535631").cloned().unwrap_or_default();
        let mut labels: Vec<String> = got.iter().map(FactoryPreset::label).collect();
        labels.sort();
        assert_eq!(labels, ["Bass › Fat", "Init"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod machine {
    #[test]
    #[ignore]
    fn index_this_machine() {
        let t0 = std::time::Instant::now();
        let index = super::build(&super::roots());
        let total: usize = index.values().map(Vec::len).sum();
        let bad = index.values().flatten().filter(|p| super::read_state(&p.path).is_none()).count();
        println!("{} plugins, {total} presets, {bad} unreadable, {} ms", index.len(), t0.elapsed().as_millis());
        super::INDEX.set(index).ok();
        for p in super::for_plugin("vst3:01EFCDAB8291EBFA4272616250535631").iter().take(5) {
            println!("PS-VC1: {}", p.label());
        }
    }
}
