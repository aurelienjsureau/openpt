use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Instant;

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("OPENPT_PROFILE").is_some())
}

thread_local! {
    static ACC: RefCell<(BTreeMap<&'static str, f64>, u32)> = const { RefCell::new((BTreeMap::new(), 0)) };
    static LAST: RefCell<Vec<(&'static str, f64)>> = const { RefCell::new(Vec::new()) };
}

pub struct Scope(&'static str, Instant);

impl Drop for Scope {
    fn drop(&mut self) {
        let ms = self.1.elapsed().as_secs_f64() * 1000.0;
        ACC.with(|a| *a.borrow_mut().0.entry(self.0).or_insert(0.0) += ms);
    }
}

pub fn scope(name: &'static str) -> Option<Scope> {
    enabled().then(|| Scope(name, Instant::now()))
}

pub fn frame() {
    if !enabled() {
        return;
    }
    ACC.with(|a| {
        let mut a = a.borrow_mut();
        a.1 += 1;
        if a.1 >= 120 {
            let n = a.1 as f64;
            let mut v: Vec<(&'static str, f64)> = a.0.iter().map(|(k, t)| (*k, t / n)).collect();
            v.sort_by(|x, y| y.1.total_cmp(&x.1));
            log::info!("profile (ms/frame): {}", v.iter().map(|(k, t)| format!("{k} {t:.2}")).collect::<Vec<_>>().join(", "));
            LAST.with(|l| *l.borrow_mut() = v);
            a.0.clear();
            a.1 = 0;
        }
    });
}

pub fn last() -> Vec<(&'static str, f64)> {
    LAST.with(|l| l.borrow().clone())
}
