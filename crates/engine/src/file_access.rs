use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

#[derive(Debug)]
pub struct AutomationRoots {
    read: Option<Root>,
    write: Option<Root>,
}

#[derive(Debug)]
struct Root {
    resolved: PathBuf,
    shown: PathBuf,
}

impl Root {
    fn open(path: &Path, access: Access) -> Result<Self, String> {
        let kind = match access {
            Access::Read => "read",
            Access::Write => "write",
        };
        let resolved = std::fs::canonicalize(path)
            .and_then(|p| if p.is_dir() { Ok(p) } else { Err(std::io::ErrorKind::NotADirectory.into()) })
            .map_err(|e| format!("cannot open the automation {kind} root `{}`: {e}", path.display()))?;
        Ok(Self { shown: plain(resolved.clone()), resolved })
    }
}

impl AutomationRoots {
    pub fn new(read: Option<&Path>, write: Option<&Path>) -> Result<Option<Arc<Self>>, String> {
        if read.is_none() && write.is_none() {
            return Ok(None);
        }
        let read = read.map(|p| Root::open(p, Access::Read)).transpose()?;
        let write = write.map(|p| Root::open(p, Access::Write)).transpose()?;
        Ok(Some(Arc::new(Self { read, write })))
    }

    pub fn read_root(&self) -> Option<&Path> {
        self.read.as_ref().map(|r| r.shown.as_path())
    }

    pub fn write_root(&self) -> Option<&Path> {
        self.write.as_ref().map(|r| r.shown.as_path())
    }

    pub fn check(&self, path: impl AsRef<Path>, access: Access) -> Result<(), String> {
        let path = path.as_ref();
        let shown_path = path.display();
        let root = match access {
            Access::Read => self.read.as_ref(),
            Access::Write => self.write.as_ref(),
        };
        let Some(Root { resolved: folder, shown }) = root else {
            return match access {
                Access::Read => Err(format!("automation filesystem access is not granted: read authority is absent: {shown_path}")),
                Access::Write => Err(format!("automation filesystem access is not granted: write authority is absent: {shown_path}")),
            };
        };
        if inside(&resolve(path)?, folder) {
            return Ok(());
        }
        let root = shown.display();
        match access {
            Access::Read => Err(format!("automation path rejected: outside the read root {root}: {shown_path}")),
            Access::Write => Err(format!("automation path rejected: outside the write root {root}: {shown_path}")),
        }
    }
}

#[derive(Clone, Debug)]
enum Scope {
    Confined(Arc<AutomationRoots>),
    Unconfined,
}

thread_local! {
    static SCOPE: RefCell<Option<Scope>> = const { RefCell::new(None) };
}

static PROCESS: OnceLock<Arc<AutomationRoots>> = OnceLock::new();

pub fn confine_process(roots: Arc<AutomationRoots>) -> bool {
    PROCESS.set(roots).is_ok()
}

struct Restore(Option<Scope>);

impl Drop for Restore {
    fn drop(&mut self) {
        let previous = self.0.take();
        SCOPE.with(|s| *s.borrow_mut() = previous);
    }
}

fn with_scope<T>(scope: Option<Scope>, f: impl FnOnce() -> T) -> T {
    let _restore = Restore(SCOPE.with(|s| s.replace(scope)));
    f()
}

pub fn confine<T>(roots: Option<&Arc<AutomationRoots>>, f: impl FnOnce() -> T) -> T {
    match roots {
        Some(r) => with_scope(Some(Scope::Confined(r.clone())), f),
        None => f(),
    }
}

pub fn unconfined<T>(f: impl FnOnce() -> T) -> T {
    with_scope(Some(Scope::Unconfined), f)
}

pub fn current() -> Option<Arc<AutomationRoots>> {
    match SCOPE.with(|s| s.borrow().clone()) {
        Some(Scope::Confined(r)) => Some(r),
        Some(Scope::Unconfined) => None,
        None => PROCESS.get().cloned(),
    }
}

fn plain(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}

pub fn check_read(path: impl AsRef<Path>) -> Result<(), String> {
    current().map_or(Ok(()), |r| r.check(path.as_ref(), Access::Read))
}

pub fn check_write(path: impl AsRef<Path>) -> Result<(), String> {
    current().map_or(Ok(()), |r| r.check(path.as_ref(), Access::Write))
}

fn resolve(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("automation path rejected: path is empty".into());
    }
    if cfg!(windows) && path.components().any(|c| matches!(c, Component::Normal(n) if is_device_name(&n.to_string_lossy()))) {
        return Err(format!("automation path rejected: reserved device names are not allowed: {}", path.display()));
    }
    let absolute = std::path::absolute(path).map_err(|e| format!("automation path rejected: cannot resolve {}: {e}", path.display()))?;
    let mut missing: Vec<OsString> = vec![];
    let mut at = absolute.as_path();
    loop {
        match std::fs::canonicalize(at) {
            Ok(mut found) => {
                found.extend(missing.iter().rev());
                return Ok(found);
            }
            Err(e) => match std::fs::symlink_metadata(at) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(format!("automation path rejected: a link that leads nowhere: {}", path.display()));
                }
                Ok(_) => return Err(format!("automation path rejected: cannot resolve {}: {e}", path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("automation path rejected: cannot resolve {}: {e}", path.display())),
            },
        }
        let parent = at.parent();
        match (at.components().next_back(), parent) {
            (Some(Component::Normal(name)), Some(parent)) => {
                missing.push(name.to_os_string());
                at = parent;
            }
            (Some(Component::ParentDir), _) => {
                return Err(format!("automation path rejected: parent traversal below a folder that doesn't exist: {}", path.display()));
            }
            _ => {
                let e = std::io::Error::from(std::io::ErrorKind::NotFound);
                return Err(format!("automation path rejected: cannot resolve {}: {e}", path.display()));
            }
        }
    }
}

fn inside(path: &Path, root: &Path) -> bool {
    let mut path = path.components();
    root.components().all(|r| path.next().is_some_and(|p| same_component(p, r)))
}

fn same_component(a: Component, b: Component) -> bool {
    if a == b {
        return true;
    }
    if !cfg!(windows) {
        return false;
    }
    match (a.as_os_str().to_str(), b.as_os_str().to_str()) {
        (Some(a), Some(b)) => a.to_lowercase() == b.to_lowercase(),
        _ => false,
    }
}

fn is_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component).trim_end_matches(' ').to_ascii_uppercase();
    let numbered = |prefix: &str| stem.strip_prefix(prefix).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$") || numbered("COM") || numbered("LPT")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(name: &str) -> (PathBuf, Arc<AutomationRoots>) {
        let base = std::env::temp_dir().join(format!("sc-file-access-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for d in ["in/sub", "out", "outside", "in2"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        std::fs::write(base.join("in/a.openpt"), "x").unwrap();
        std::fs::write(base.join("outside/secret.openpt"), "x").unwrap();
        let roots = AutomationRoots::new(Some(&base.join("in")), Some(&base.join("out"))).unwrap().unwrap();
        (base, roots)
    }

    fn s(p: PathBuf) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn paths_inside_the_roots_pass_and_others_dont() {
        let (base, roots) = folders("inside");
        assert_eq!(roots.check(s(base.join("in/a.openpt")), Access::Read), Ok(()));
        assert_eq!(roots.check(s(base.join("in")), Access::Read), Ok(()), "the root itself");
        assert_eq!(roots.check(s(base.join("in/sub/new.openpt")), Access::Read), Ok(()), "missing files are checked by their folder");
        assert_eq!(roots.check(s(base.join("out/new/deeper/x.wav")), Access::Write), Ok(()), "and by their nearest folder");
        let e = roots.check(s(base.join("outside/secret.openpt")), Access::Read).unwrap_err();
        assert!(e.starts_with("automation path rejected: outside the read root"), "{e}");
        let e = roots.check(s(base.join("in/a.openpt")), Access::Write).unwrap_err();
        assert!(e.contains("outside the write root"), "{e}");
        assert!(roots.check(s(base.join("out/x.wav")), Access::Read).unwrap_err().contains("outside the read root"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_sibling_with_the_same_prefix_is_outside() {
        let (base, roots) = folders("prefix");
        std::fs::write(base.join("in2/b.openpt"), "x").unwrap();
        assert!(roots.check(s(base.join("in2/b.openpt")), Access::Read).is_err(), "in2 is not inside in");
        assert!(roots.check(format!("{}2/c.openpt", s(base.join("in"))), Access::Read).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn parent_traversal_is_resolved_or_refused() {
        let (base, roots) = folders("dotdot");
        let sep = std::path::MAIN_SEPARATOR;
        let up = format!("{}{sep}..{sep}outside{sep}secret.openpt", s(base.join("in")));
        assert!(roots.check(up, Access::Read).unwrap_err().contains("outside the read root"), "`..` out of the root");
        let back = format!("{}{sep}..{sep}a.openpt", s(base.join("in/sub")));
        assert_eq!(roots.check(back, Access::Read), Ok(()), "`..` that stays inside");
        let missing = format!("{}{sep}nope{sep}..{sep}..{sep}outside{sep}x.openpt", s(base.join("out")));
        let e = roots.check(missing, Access::Write).unwrap_err();
        assert!(e.contains("parent traversal below a folder that doesn't exist") || e.contains("outside the write root"), "{e}");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn relative_paths_resolve_against_the_working_directory() {
        let (base, roots) = folders("relative");
        assert!(roots.check("a.openpt", Access::Read).unwrap_err().contains("outside the read root"));
        assert!(roots.check("", Access::Read).unwrap_err().contains("path is empty"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_missing_root_is_an_error_and_none_is_no_confinement() {
        let (base, _) = folders("missing");
        let e = AutomationRoots::new(Some(&base.join("nope")), None).unwrap_err();
        assert!(e.starts_with("cannot open the automation read root"), "{e}");
        let e = AutomationRoots::new(None, Some(&base.join("in/a.openpt"))).unwrap_err();
        assert!(
            e.starts_with("cannot open the automation write root")
                && e.ends_with(&std::io::Error::from(std::io::ErrorKind::NotADirectory).to_string()),
            "{e}"
        );
        assert!(AutomationRoots::new(Some(Path::new("")), None).unwrap_err().starts_with("cannot open the automation read root ``"));
        assert!(AutomationRoots::new(None, None).unwrap().is_none());
        let read_only = AutomationRoots::new(Some(&base.join("in")), None).unwrap().unwrap();
        let e = read_only.check(s(base.join("in/x.openpt")), Access::Write).unwrap_err();
        assert!(e.contains("write authority is absent"), "{e}");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn links_are_followed_before_comparing() {
        let (base, roots) = folders("links");
        #[cfg(unix)]
        let made = |target: &Path, link: &Path| std::os::unix::fs::symlink(target, link).is_ok();
        #[cfg(windows)]
        let made = |target: &Path, link: &Path| {
            let made =
                if target.is_dir() { std::os::windows::fs::symlink_dir(target, link) } else { std::os::windows::fs::symlink_file(target, link) };
            made.is_ok()
        };
        #[cfg(not(any(unix, windows)))]
        let made = |_: &Path, _: &Path| false;
        if !made(&base.join("outside/secret.openpt"), &base.join("in/leak.openpt")) {
            eprintln!("skipped: symbolic links can't be made here");
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        assert!(roots.check(s(base.join("in/leak.openpt")), Access::Read).unwrap_err().contains("outside the read root"), "a file link out");
        assert!(made(&base.join("outside"), &base.join("out/away")));
        let e = roots.check(s(base.join("out/away/new.wav")), Access::Write).unwrap_err();
        assert!(e.contains("outside the write root"), "a folder link out: {e}");
        assert!(made(&base.join("outside/gone.openpt"), &base.join("out/dangling.wav")));
        let e = roots.check(s(base.join("out/dangling.wav")), Access::Write).unwrap_err();
        assert!(e.contains("a link that leads nowhere"), "writing would create the file outside: {e}");
        assert!(made(&base.join("in/sub"), &base.join("in/alias")));
        assert_eq!(roots.check(s(base.join("in/alias/x.openpt")), Access::Read), Ok(()));
        assert!(made(&base.join("in"), &base.join("in-link")));
        let linked = AutomationRoots::new(Some(&base.join("in-link")), None).unwrap().unwrap();
        assert_eq!(linked.check(s(base.join("in/a.openpt")), Access::Read), Ok(()));
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    fn windows_ignores_case_and_refuses_devices() {
        let (base, roots) = folders("case");
        let upper = s(base.join("IN/a.openpt"));
        assert_eq!(roots.check(&upper, Access::Read), Ok(()), "{upper}");
        assert_eq!(roots.check(s(base.join("In/Sub/New.openpt")), Access::Read), Ok(()));
        let e = roots.check(s(base.join("out/NUL")), Access::Write).unwrap_err();
        assert!(e.contains("reserved device names"), "{e}");
        assert!(roots.check(s(base.join("out/com1.txt")), Access::Write).is_err());
        assert_eq!(roots.check(s(base.join("out/console.txt")), Access::Write), Ok(()));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn scopes_nest_and_stay_on_their_thread() {
        let (base, roots) = folders("scopes");
        let outside = s(base.join("outside/secret.openpt"));
        assert_eq!(check_read(&outside), Ok(()), "nothing is confined by default");
        confine(Some(&roots), || {
            assert!(check_read(&outside).is_err());
            assert_eq!(unconfined(|| check_read(&outside)), Ok(()));
            assert!(check_read(&outside).is_err(), "restored after unconfined");
            let o = outside.clone();
            let there = std::thread::spawn(move || check_read(&o).is_ok()).join().unwrap();
            assert!(there, "another thread is not confined by this thread's scope");
        });
        assert_eq!(check_read(&outside), Ok(()), "restored after confine");
        let r = std::panic::catch_unwind(|| confine(Some(&roots), || panic!("boom")));
        assert!(r.is_err());
        assert_eq!(check_read(&outside), Ok(()), "restored after a panic");
        assert_eq!(confine(None, || check_read(&outside)), Ok(()));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn device_names() {
        for d in ["CON", "nul", "Com1", "LPT9.txt", "aux.openpt", "NUL "] {
            assert!(is_device_name(d), "{d}");
        }
        for f in ["console", "com10", "COM0", "lpt", "a.openpt", "nul-x"] {
            assert!(!is_device_name(f), "{f}");
        }
    }

    #[test]
    fn commands_do_not_touch_the_file_system_themselves() {
        fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, files);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    files.push(path);
                }
            }
        }
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![];
        walk(&src, &mut files);
        assert!(files.iter().any(|f| f.ends_with("lib.rs")) && files.iter().any(|f| f.ends_with("cmd/file.rs")), "{files:?}");
        let mut offenders = vec![];
        for path in files {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.parent() == Some(src.as_path()) && matches!(name, "io.rs" | "file_access.rs" | "autosave.rs") || name.starts_with("tests") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let code = text.split("#[cfg(test)]").next().unwrap_or("");
            for (n, line) in code.lines().enumerate() {
                let line = line.trim_start();
                let touches =
                    ["std::fs", "fs::", "File::open", "File::create", "OpenOptions", ".is_file()", ".exists()", ".is_dir()", "canonicalize"]
                        .iter()
                        .any(|w| line.contains(w));
                if !line.starts_with("//") && touches {
                    offenders.push(format!("{}:{}: {line}", path.display(), n + 1));
                }
            }
        }
        assert!(offenders.is_empty(), "use the checked helpers in crate::io (read_file, write_file, ...): {offenders:#?}");
    }
}
