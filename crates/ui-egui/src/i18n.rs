use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub const LANGUAGES: [(&str, &str); 2] = [("en", "English"), ("fr", "Français")];

const CATALOGS: [(&str, &str); 1] = [("fr", include_str!("../locales/fr.tsv"))];

static CURRENT: AtomicU8 = AtomicU8::new(0);
static FOLLOW_SYSTEM: AtomicBool = AtomicBool::new(false);

fn unescape(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\t", "\t")
}

pub fn parse(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (unescape(k), unescape(v.trim_end_matches('\r'))))
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .collect()
}

fn catalogs() -> &'static HashMap<&'static str, HashMap<String, String>> {
    static MAPS: OnceLock<HashMap<&'static str, HashMap<String, String>>> = OnceLock::new();
    MAPS.get_or_init(|| CATALOGS.iter().map(|(code, text)| (*code, parse(text))).collect())
}

#[cfg(test)]
fn french() -> &'static HashMap<String, String> {
    static EMPTY: OnceLock<HashMap<String, String>> = OnceLock::new();
    catalogs().get("fr").unwrap_or_else(|| EMPTY.get_or_init(HashMap::new))
}

pub fn set_language(code: &str) {
    let index = LANGUAGES.iter().position(|(c, _)| *c == code).unwrap_or(0);
    CURRENT.store(u8::try_from(index).unwrap_or(0), Ordering::Relaxed);
}

pub fn language() -> &'static str {
    LANGUAGES.get(usize::from(CURRENT.load(Ordering::Relaxed))).map_or("en", |(c, _)| c)
}

pub fn system_language() -> &'static str {
    static SYSTEM: OnceLock<&'static str> = OnceLock::new();
    SYSTEM.get_or_init(|| {
        let locale = sys_locale::get_locale().unwrap_or_default().to_ascii_lowercase();
        LANGUAGES.iter().map(|(c, _)| *c).find(|c| locale.starts_with(c)).unwrap_or("en")
    })
}

pub fn follow_system_language() {
    FOLLOW_SYSTEM.store(true, Ordering::Relaxed);
}

pub fn default_language() -> &'static str {
    if FOLLOW_SYSTEM.load(Ordering::Relaxed) { system_language() } else { "en" }
}

pub fn tr(s: &str) -> &str {
    if CURRENT.load(Ordering::Relaxed) == 0 {
        return s;
    }
    catalogs().get(language()).and_then(|m| m.get(s)).map_or(s, String::as_str)
}

pub fn tr_path(path: &str) -> String {
    path.split(" > ").map(tr).collect::<Vec<_>>().join(" > ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_parses_and_falls_back_to_english() {
        let m = parse("# comment\nOpen\tOuvrir\nLine\\nBreak\tLigne\\nCoupure\nEmpty\t\n");
        assert_eq!(m.get("Open").map(String::as_str), Some("Ouvrir"));
        assert_eq!(m.get("Line\nBreak").map(String::as_str), Some("Ligne\nCoupure"));
        assert!(!m.contains_key("Empty"));
        assert!(!french().is_empty());
    }

    fn literals(src: &str, out: &mut std::collections::BTreeSet<String>) {
        let mut rest = src;
        while let Some(at) = rest.find("t!(\"") {
            let macro_call = rest[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric() && c != '_');
            rest = &rest[at + 4..];
            if !macro_call {
                continue;
            }
            let mut s = String::new();
            let mut chars = rest.char_indices();
            while let Some((i, c)) = chars.next() {
                match c {
                    '\\' => match chars.next().map(|(_, e)| e) {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some(e) => s.push(e),
                        None => {}
                    },
                    '"' => {
                        rest = &rest[i..];
                        break;
                    }
                    _ => s.push(c),
                }
            }
            out.insert(s);
        }
    }

    const HELPERS: [&str; 17] = [
        "group(",
        "check(",
        "row(",
        "combo(",
        "radios(",
        "number(",
        "win(",
        "section(",
        "panel_header(",
        "icon_button(",
        "text_toggle(",
        "rec_toggle(",
        "selector_box_with_tooltip(",
        "pan_knob(",
        "knob(",
        "focus_button(",
        "menu_row(",
    ];

    fn display_text(s: &str) -> bool {
        s.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 2
            && !s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.' || c == '-')
            && !s.contains('_')
            && !s.ends_with('\\')
            && !s.contains('{')
    }

    fn helper_literals(src: &str, out: &mut std::collections::BTreeSet<String>) {
        for h in HELPERS {
            let mut rest = src;
            while let Some(at) = rest.find(h) {
                let ident = rest[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric() && c != '_');
                rest = &rest[at + h.len()..];
                if !ident {
                    continue;
                }
                let (mut depth, mut end, mut in_str, mut esc) = (1usize, rest.len(), false, false);
                for (i, c) in rest.char_indices() {
                    match (in_str, c) {
                        (true, '\\') if !esc => esc = true,
                        (true, '"') if !esc => in_str = false,
                        (true, _) => esc = false,
                        (false, '"') => in_str = true,
                        (false, '(') => depth += 1,
                        (false, ')') => {
                            depth -= 1;
                            if depth == 0 {
                                end = i;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let args = &rest[..end];
                let mut parts = args.split('"');
                parts.next();
                while let Some(s) = parts.next() {
                    if display_text(s) {
                        out.insert(s.replace("\\n", "\n"));
                    }
                    parts.next();
                }
            }
        }
    }

    fn sources(dir: &std::path::Path, out: &mut std::collections::BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let src = std::fs::read_to_string(&path).unwrap();
                literals(&src, out);
                helper_literals(&src, out);
            }
        }
    }

    fn menu_labels(nodes: &[crate::menus::MenuNode], out: &mut std::collections::BTreeSet<String>) {
        for n in nodes {
            out.insert(n.label.clone());
            menu_labels(&n.children, out);
        }
    }

    #[test]
    fn french_covers_every_label() {
        let mut wanted = std::collections::BTreeSet::new();
        sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut wanted);
        menu_labels(&crate::menus::tree(), &mut wanted);
        for c in openpt_engine::command_specs() {
            wanted.insert(c.label.to_string());
            wanted.extend(c.menu.iter().map(|m| m.to_string()));
        }
        for (_, label, path, _) in crate::menus::UI_COMMANDS {
            wanted.insert(label.to_string());
            wanted.extend(path.split(" > ").filter(|p| !p.is_empty()).map(str::to_string));
        }
        let map = french();
        let missing: Vec<&String> = wanted.iter().filter(|w| w.chars().any(char::is_alphabetic) && !map.contains_key(*w)).collect();
        assert!(
            missing.is_empty(),
            "{} labels have no French translation in locales/fr.tsv:\n{}",
            missing.len(),
            missing.iter().map(|m| m.replace('\n', "\\n")).collect::<Vec<_>>().join("\n")
        );
    }
}
