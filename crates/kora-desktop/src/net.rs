//! Talking to the panel: asking `kora` for a signed-in panel, reading its event stream, and the few API calls the
//! tray makes. Only ever 127.0.0.1, with the panel's token in an `Authorization` header (never in an address).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kora_shell::model::{AlertsList, SelfUpdate, ServersList};
use kora_shell::session::{self, Outcome};
use kora_shell::tray::Verb;
use kora_shell::{nav, sse};
use tao::event_loop::EventLoopProxy;

use crate::app::UserEvent;
use crate::os;

/// How to call the panel.
#[derive(Clone)]
pub struct Creds {
    pub port: u16,
    pub token: String,
}

fn agent(read: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(2)).timeout_read(read).timeout_write(Duration::from_secs(5)).build()
}

fn url(c: &Creds, path: &str) -> String {
    format!("http://127.0.0.1:{}/api/v1{path}", c.port)
}

/// The message of a panel error (`{"error":{"message":"..."}}`), else the status.
fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v["error"]["message"].as_str().map(str::to_string)).unwrap_or_else(|| format!("the panel answered {code}"))
        }
        ureq::Error::Transport(t) => t.to_string(),
    }
}

fn get(c: &Creds, path: &str) -> Result<String, String> {
    agent(Duration::from_secs(10)).get(&url(c, path)).set("Authorization", &format!("Bearer {}", c.token)).set("X-Kora", "1").call().map_err(describe)?.into_string().map_err(|e| e.to_string())
}

fn post(c: &Creds, path: &str, body: &str) -> Result<String, String> {
    agent(Duration::from_secs(20))
        .post(&url(c, path))
        .set("Authorization", &format!("Bearer {}", c.token))
        .set("X-Kora", "1")
        .set("Content-Type", "application/json")
        .send_string(body)
        .map_err(describe)?
        .into_string()
        .map_err(|e| e.to_string())
}

/// Start, stop or restart a server (the panel runs it as a job; the event stream shows the result).
pub fn control(c: &Creds, server: &str, verb: Verb) -> Result<(), String> {
    post(c, &format!("/servers/{}/control", nav::encode_segment(server)), &format!("{{\"action\":\"{}\"}}", verb.api_word())).map(|_| ())
}

/// A fresh one-time sign-in link (the one from the first answer is used once and expires).
pub fn fresh_link(c: &Creds) -> Result<String, String> {
    let body = post(c, "/session/link", "{}")?;
    session::code_from_json(&body).map(|code| session::code_link(c.port, &code)).ok_or_else(|| "the panel gave no sign-in code".to_string())
}

/// The version Kora could update to, if any.
pub fn self_update(c: &Creds) -> Result<Option<String>, String> {
    let u: SelfUpdate = serde_json::from_str(&get(c, "/self-update")?).map_err(|e| e.to_string())?;
    Ok(if u.available { u.latest } else { None })
}

/// Ask `kora` for the panel (see `kora desktop session`).
fn ask_session(kora: &Path, no_background: bool) -> Outcome {
    let mut c = Command::new(kora);
    c.arg("--no-elevate").args(["desktop", "session"]);
    if no_background {
        c.arg("--no-background");
    }
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    os::hidden(&mut c);
    match c.output() {
        Ok(o) => session::parse_output(&String::from_utf8_lossy(&o.stdout)),
        Err(e) => Outcome::Failed(format!("Kora could not be started ({e}). Is it installed next to this app?")),
    }
}

/// No background service is possible: run the panel ourselves, in the background of this app.
fn start_foreground_panel(kora: &Path, slot: &Mutex<Option<Child>>) {
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(child) = guard.as_mut() {
        if matches!(child.try_wait(), Ok(None)) {
            return; // still running
        }
    }
    let mut c = Command::new(kora);
    c.arg("--no-elevate").args(["panel", "--foreground", "--no-open"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    os::hidden(&mut c);
    *guard = c.spawn().ok();
}

fn send(proxy: &Mutex<EventLoopProxy<UserEvent>>, e: UserEvent) {
    if let Ok(p) = proxy.lock() {
        let _ = p.send_event(e);
    }
}

/// The supervisor: get a signed-in panel, watch it, and start over when it goes away (restart, new port).
pub fn spawn(proxy: EventLoopProxy<UserEvent>, kora: PathBuf, no_background: bool, foreground: Arc<Mutex<Option<Child>>>) {
    let proxy = Arc::new(Mutex::new(proxy));
    std::thread::spawn(move || {
        let mut attempt = 0u32;
        loop {
            match ask_session(&kora, no_background) {
                Outcome::Ready(s) => {
                    attempt = 0;
                    let creds = Creds { port: s.port, token: s.token.clone() };
                    send(&proxy, UserEvent::Session { port: s.port, link: s.link.clone(), creds: creds.clone() });
                    let stop = Arc::new(AtomicBool::new(false));
                    spawn_poller(creds.clone(), proxy.clone(), stop.clone());
                    let why = stream(&creds, &proxy);
                    stop.store(true, Ordering::Relaxed);
                    send(&proxy, UserEvent::Connected(false));
                    let _ = why; // the reason is not shown: the tray says "waiting for Kora's panel"
                }
                Outcome::NeedForeground => {
                    start_foreground_panel(&kora, &foreground);
                    attempt = attempt.min(2);
                }
                Outcome::Failed(m) => send(&proxy, UserEvent::SessionFailed(m)),
            }
            std::thread::sleep(Duration::from_secs(session::backoff_secs(attempt)));
            attempt += 1;
        }
    });
}

/// Read `GET /events` until it ends. Every `servers` event carries the whole list.
fn stream(c: &Creds, proxy: &Mutex<EventLoopProxy<UserEvent>>) -> Result<(), String> {
    let resp = agent(Duration::from_secs(45)).get(&url(c, "/events")).set("Authorization", &format!("Bearer {}", c.token)).set("X-Kora", "1").set("Accept", "text/event-stream").call().map_err(describe)?;
    send(proxy, UserEvent::Connected(true));
    let mut reader = BufReader::new(resp.into_reader());
    let mut parser = sse::Parser::default();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("the panel closed the connection".into());
        }
        if let Some(ev) = parser.line(line.trim_end_matches('\n')) {
            if ev.name == "servers" {
                if let Ok(l) = serde_json::from_str::<ServersList>(&ev.data) {
                    send(proxy, UserEvent::Servers(l.servers));
                }
            }
        }
    }
}

/// Alerts every few seconds, the Kora update check now and then.
fn spawn_poller(c: Creds, proxy: Arc<Mutex<EventLoopProxy<UserEvent>>>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut tick = 0u64;
        while !stop.load(Ordering::Relaxed) {
            if tick % 8 == 0 {
                if let Ok(body) = get(&c, "/alerts") {
                    if let Ok(l) = serde_json::from_str::<AlertsList>(&body) {
                        send(&proxy, UserEvent::Alerts(l.alerts));
                    }
                }
            }
            // Twenty seconds after the start, then every six hours.
            if tick == 20 || (tick > 20 && tick % (6 * 3600) == 20) {
                if let Ok(u) = self_update(&c) {
                    send(&proxy, UserEvent::Update(u));
                }
            }
            std::thread::sleep(Duration::from_secs(1));
            tick += 1;
        }
    });
}
