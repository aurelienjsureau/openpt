use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

const MAX_CHILD_OUTPUT: u64 = 1 << 20;
const MAX_CACHE_BYTES: u64 = 16 << 20;
const CACHE_MAGIC: &str = "SCV3SCAN 1";

#[derive(Debug, Clone)]
pub struct Scanner {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub cache: Option<PathBuf>,
    pub timeout: Duration,
    pub jobs: usize,
}

static SCANNER: OnceLock<Scanner> = OnceLock::new();

pub fn set_scanner(s: Scanner) -> bool {
    SCANNER.set(s).is_ok()
}

pub(crate) fn scanner() -> Option<&'static Scanner> {
    SCANNER.get()
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawClass {
    pub class_id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub sdk_version: String,
    pub sub_categories: String,
}

fn clean(s: &str) -> String {
    s.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).collect()
}

fn class_line(c: &RawClass) -> String {
    format!(
        "C\t{}\t{}\t{}\t{}\t{}\t{}",
        clean(&c.class_id),
        clean(&c.name),
        clean(&c.vendor),
        clean(&c.version),
        clean(&c.sdk_version),
        clean(&c.sub_categories)
    )
}

fn parse_class(line: &str) -> Option<RawClass> {
    let mut f = line.split('\t');
    if f.next()? != "C" {
        return None;
    }
    let class_id = f.next()?.to_string();
    if class_id.len() != 32 || !class_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(RawClass {
        class_id,
        name: f.next()?.to_string(),
        vendor: f.next()?.to_string(),
        version: f.next()?.to_string(),
        sdk_version: f.next()?.to_string(),
        sub_categories: f.next().unwrap_or("").to_string(),
    })
}

pub fn scan_child(path: &Path) -> i32 {
    match crate::registry::load_bundle(path) {
        Ok(ds) => {
            let mut out = std::io::stdout().lock();
            for d in ds {
                let raw = RawClass {
                    class_id: d.class_id,
                    name: d.name,
                    vendor: d.vendor,
                    version: d.version,
                    sdk_version: d.sdk_version,
                    sub_categories: d.sub_categories.join("|"),
                };
                if writeln!(out, "{}", class_line(&raw)).is_err() {
                    return 1;
                }
            }
            let _ = writeln!(out, "END");
            let _ = out.flush();
            0
        }
        Err(e) => {
            println!("ERR\t{}", clean(&e.to_string()));
            1
        }
    }
}

fn stamp(bundle: &Path) -> Option<String> {
    let bin = crate::scan::bundle_binary(bundle)?;
    let m = std::fs::metadata(bin).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(format!("{}.{}:{}", t.as_secs(), t.subsec_nanos(), m.len()))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    Loaded(Vec<RawClass>),
    Failed(String),
}

type Cache = Vec<(PathBuf, String, Outcome)>;

fn read_cache(path: &Path) -> Cache {
    let mut text = String::new();
    let Ok(f) = std::fs::File::open(path) else { return Vec::new() };
    if f.take(MAX_CACHE_BYTES).read_to_string(&mut text).is_err() {
        return Vec::new();
    }
    let mut lines = text.lines();
    if lines.next() != Some(CACHE_MAGIC) {
        return Vec::new();
    }
    let mut out: Cache = Vec::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix("B\t") {
            let mut f = rest.split('\t');
            let (Some(p), Some(st), Some(kind)) = (f.next(), f.next(), f.next()) else { continue };
            let outcome = if kind == "ok" { Outcome::Loaded(Vec::new()) } else { Outcome::Failed(f.next().unwrap_or("").to_string()) };
            out.push((PathBuf::from(p), st.to_string(), outcome));
        } else if let Some(c) = parse_class(line)
            && let Some((_, _, Outcome::Loaded(v))) = out.last_mut()
        {
            v.push(c);
        }
    }
    out
}

fn write_cache(path: &Path, entries: &[(PathBuf, String, Outcome)]) {
    let mut s = String::from(CACHE_MAGIC);
    s.push('\n');
    for (p, st, o) in entries {
        match o {
            Outcome::Loaded(classes) => {
                s.push_str(&format!("B\t{}\t{st}\tok\n", clean(&p.to_string_lossy())));
                for c in classes {
                    s.push_str(&class_line(c));
                    s.push('\n');
                }
            }
            Outcome::Failed(why) => s.push_str(&format!("B\t{}\t{st}\tfail\t{}\n", clean(&p.to_string_lossy()), clean(why))),
        }
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, s).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

pub(crate) fn clear_cache() {
    if let Some(c) = scanner().and_then(|s| s.cache.as_ref()) {
        let _ = std::fs::remove_file(c);
    }
}

fn run_child(sc: &Scanner, bundle: &Path) -> Outcome {
    let mut cmd = Command::new(&sc.command);
    cmd.args(&sc.args).arg(bundle).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Outcome::Failed(format!("scanner did not start: {e}")),
    };
    let reader = child.stdout.take().map(|out| {
        std::thread::spawn(move || {
            let mut lines = Vec::new();
            for l in BufReader::new(out.take(MAX_CHILD_OUTPUT)).lines().map_while(Result::ok) {
                lines.push(l);
            }
            lines
        })
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() > sc.timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break None,
        }
    };
    let lines = reader.and_then(|h| h.join().ok()).unwrap_or_default();
    match status {
        None => Outcome::Failed(format!("hung while loading (over {} s)", sc.timeout.as_secs())),
        Some(s) if lines.iter().any(|l| l == "END") => {
            let _ = s;
            Outcome::Loaded(lines.iter().filter_map(|l| parse_class(l)).collect())
        }
        Some(s) => {
            let why = lines.iter().find_map(|l| l.strip_prefix("ERR\t")).map(str::to_string);
            Outcome::Failed(why.unwrap_or_else(|| match s.code() {
                Some(c) => format!("crashed while loading (exit code {:#x})", c as u32),
                None => "crashed while loading".to_string(),
            }))
        }
    }
}

pub(crate) fn scan(sc: &Scanner, bundles: &[PathBuf]) -> Vec<(PathBuf, Outcome)> {
    let cached = sc.cache.as_deref().map(read_cache).unwrap_or_default();
    let stamps: Vec<Option<String>> = bundles.iter().map(|b| stamp(b)).collect();
    let results: Vec<Mutex<Option<Outcome>>> = bundles
        .iter()
        .zip(&stamps)
        .map(|(b, st)| Mutex::new(cached.iter().find(|(p, s, _)| p == b && Some(s) == st.as_ref()).map(|(_, _, o)| o.clone())))
        .collect();
    let todo: Vec<usize> =
        results.iter().enumerate().filter(|(_, r)| r.lock().unwrap_or_else(PoisonError::into_inner).is_none()).map(|(i, _)| i).collect();
    if !todo.is_empty() {
        log::info!("VST3 scan: opening {} of {} bundles in scanner processes", todo.len(), bundles.len());
    }
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..sc.jobs.clamp(1, 32).min(todo.len().max(1)) {
            s.spawn(|| {
                while let Some(&i) = todo.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if let (Some(b), Some(slot)) = (bundles.get(i), results.get(i)) {
                        let o = run_child(sc, b);
                        if let Outcome::Failed(why) = &o {
                            log::warn!("VST3 {}: {why}; skipped until it changes or you rescan", b.display());
                        }
                        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(o);
                    }
                }
            });
        }
    });
    let out: Vec<(PathBuf, Outcome)> = bundles
        .iter()
        .zip(results)
        .map(|(b, r)| (b.clone(), r.into_inner().unwrap_or_else(PoisonError::into_inner).unwrap_or(Outcome::Failed("not scanned".into()))))
        .collect();
    if let Some(c) = &sc.cache {
        let entries: Cache = out.iter().zip(stamps).filter_map(|((p, o), st)| st.map(|st| (p.clone(), st, o.clone()))).collect();
        write_cache(c, &entries);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: char) -> RawClass {
        RawClass {
            class_id: std::iter::repeat_n(id, 32).collect(),
            name: "Comp\tX".into(),
            vendor: "V".into(),
            version: "1.0".into(),
            sdk_version: "VST 3.7".into(),
            sub_categories: "Fx|Dynamics".into(),
        }
    }

    #[test]
    fn class_lines_round_trip_and_reject_garbage() {
        let c = raw('A');
        let back = parse_class(&class_line(&c)).unwrap();
        assert_eq!(back.name, "Comp X");
        assert_eq!(back.sub_categories, "Fx|Dynamics");
        assert_eq!(parse_class("C\tnothex\ta\tb\tc\td\te"), None);
        assert_eq!(parse_class("X\tAAAA"), None);
        assert_eq!(parse_class(""), None);
    }

    #[test]
    fn cache_round_trips_and_ignores_other_files() {
        let dir = std::env::temp_dir().join(format!("scv3-cache-test-{}", std::process::id()));
        let path = dir.join("scan.tsv");
        let entries = vec![
            (PathBuf::from("/a/One.vst3"), "1.2:3".to_string(), Outcome::Loaded(vec![raw('A'), raw('B')])),
            (PathBuf::from("/b/Bad.vst3"), "4.5:6".to_string(), Outcome::Failed("crashed".into())),
        ];
        write_cache(&path, &entries);
        let back = read_cache(&path);
        assert_eq!(back.len(), 2);
        assert!(matches!(&back[0].2, Outcome::Loaded(v) if v.len() == 2));
        assert_eq!(back[1].2, Outcome::Failed("crashed".into()));
        std::fs::write(&path, "garbage\nB\tx\ty\tok\n").unwrap();
        assert!(read_cache(&path).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
