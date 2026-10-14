#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod mcp;

use openpt_engine::Engine;
use openpt_engine::file_access::{self, AutomationRoots};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

pub trait Backend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    fn has_ui(&self) -> bool;
    fn describe(&self) -> String;
}

pub struct Headless {
    pub engine: Engine,
    roots: Option<Arc<AutomationRoots>>,
}

impl Headless {
    pub fn new(engine: Engine) -> Self {
        Headless { engine, roots: None }
    }

    pub fn with_automation_roots(mut self, roots: Option<Arc<AutomationRoots>>) -> Self {
        self.roots = roots;
        self
    }

    fn answer(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "engine.execute" | "command" => {
                let id = params.get("command").or_else(|| params.get("id")).and_then(Value::as_str).ok_or("`command` required")?;
                let p = params.get("params").cloned().unwrap_or(json!({}));
                let r = self.engine.execute(id, &p).map_err(|e| e.to_string());
                self.engine.transport_requests.clear();
                r
            }
            "engine.commands" => self.engine.execute("engine.commands", &params).map_err(|e| e.to_string()),
            "engine.parity" => Ok(openpt_engine::catalog::parity_json()),
            "session.inspect" | "document.inspect" => {
                Ok(openpt_engine::inspect::session(&self.engine, params.get("detail").and_then(Value::as_str) == Some("full")))
            }
            m if m.starts_with("ui.") => {
                Err(format!("`{m}` needs the desktop app: start `openpt --control 0` and use `openpt-cli mcp --connect PORT`"))
            }
            other => Err(format!("unknown method `{other}`")),
        }
    }
}

impl Backend for Headless {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let roots = self.roots.clone();
        file_access::confine(roots.as_ref(), || self.answer(method, params))
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        format!("headless engine, session `{}`", self.engine.session().name)
    }
}

pub struct Remote {
    addr: String,
    conn: Option<(TcpStream, BufReader<TcpStream>)>,
    next_id: u64,
}

impl Remote {
    pub fn new(target: &str) -> Self {
        let addr = if target.contains(':') { target.to_string() } else { format!("127.0.0.1:{target}") };
        Remote { addr, conn: None, next_id: 1 }
    }

    fn connect(&mut self) -> Result<(), String> {
        if self.conn.is_some() {
            return Ok(());
        }
        let sock = self.addr.parse().map_err(|e| format!("{}: {e}", self.addr))?;
        let s = TcpStream::connect_timeout(&sock, Duration::from_millis(800)).map_err(|e| format!("cannot reach OpenPT at {}: {e}", self.addr))?;
        s.set_read_timeout(Some(Duration::from_secs(120))).map_err(|e| e.to_string())?;
        let r = s.try_clone().map_err(|e| e.to_string())?;
        self.conn = Some((s, BufReader::new(r)));
        Ok(())
    }

    fn roundtrip(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        self.connect()?;
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        let (w, r) = self.conn.as_mut().ok_or("not connected")?;
        writeln!(w, "{line}").map_err(|e| e.to_string())?;
        let mut buf = String::new();
        r.read_line(&mut buf).map_err(|e| e.to_string())?;
        if buf.is_empty() {
            return Err("connection closed".into());
        }
        let v: Value = serde_json::from_str(&buf).map_err(|e| e.to_string())?;
        if v.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(v.get("error").and_then(Value::as_str).unwrap_or("error").to_string())
        }
    }
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match self.roundtrip(method, &params) {
            Ok(v) => Ok(v),
            Err(e) if e.contains("closed") || e.contains("Broken pipe") || e.contains("reset") => {
                self.conn = None;
                self.roundtrip(method, &params)
            }
            Err(e) => Err(e),
        }
    }
    fn has_ui(&self) -> bool {
        true
    }
    fn describe(&self) -> String {
        format!("OpenPT app at {}", self.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_executes_and_inspects() {
        let mut h = Headless::new(Engine::default());
        let r = h.call("engine.execute", json!({"command": "track.new", "params": {"count": 2, "format": "stereo"}}));
        assert!(r.is_ok(), "{r:?}");
        let s = h.call("session.inspect", json!({})).unwrap();
        assert_eq!(s["tracks"].as_array().map(Vec::len), Some(2));
        assert!(h.call("ui.click", json!({})).is_err());
        assert!(h.call("nope", json!({})).is_err());
    }

    #[test]
    fn remote_reports_unreachable() {
        let mut r = Remote::new("127.0.0.1:1");
        assert!(r.call("session.inspect", json!({})).is_err());
    }
}
