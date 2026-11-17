//! `kora desktop session`: what the Kora desktop app (tray icon and window, `crates/kora-desktop`) asks of `kora`.
//!
//! The app is a thin client of the panel and must not copy the logic that finds or starts it, so it runs this
//! and reads ONE line of JSON from standard output. The decision is the one bare `kora` makes ([`bare::decide`]):
//!
//! * the panel already runs: its port and a one-time sign-in link (`mode: "running"`);
//! * it does not and a background service is possible: set it up, start it, wait for it (`mode: "started"`);
//! * no background service here (or `--no-background`, which the CI smoke test uses so it leaves no service
//!   behind): `mode: "foreground_needed"`, and the app runs `kora panel --foreground --no-open` itself;
//! * from an administrator/root session: `ok: false` with the plain explanation (the panel never runs that way).
//!
//! The JSON carries the panel's token (the app calls the panel's API with it). Standard output goes only to
//! the app that started this; the line is never logged. Nothing here needs administrator rights.

use clap::{Args, Subcommand};
use kora_core::monitor::panel_service as svc;
use kora_web::launch::SystemPorts;
use serde_json::{json, Value};

use crate::cmd::bare::{self, Plan};
use crate::cmd::panel_running::one_time_link;

#[derive(Subcommand)]
pub(crate) enum DesktopCmd {
    /// Make sure the panel runs and print one line of JSON: its port, a one-time sign-in link and its token.
    Session(SessionArgs),
}

#[derive(Args)]
pub(crate) struct SessionArgs {
    /// Never set up or start a background service (the app runs the panel itself instead).
    #[arg(long)]
    no_background: bool,
}

/// What the app is told.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Ready { mode: &'static str, port: u16, link: String, token: String },
    ForegroundNeeded { why: Option<String> },
    Failed(String),
}

impl Answer {
    pub(crate) fn to_json(&self) -> Value {
        match self {
            Answer::Ready { mode, port, link, token } => json!({ "ok": true, "mode": mode, "port": port, "link": link, "token": token }),
            Answer::ForegroundNeeded { why } => json!({ "ok": true, "mode": "foreground_needed", "message": why }),
            Answer::Failed(m) => json!({ "ok": false, "message": m }),
        }
    }
}

/// The token in a `http://127.0.0.1:<port>/#token=<token>` link.
fn token_of(link: &str) -> Option<String> {
    link.split_once("#token=").map(|(_, t)| t.to_string())
}

fn ready(mode: &'static str, port: u16, link: &str) -> Answer {
    match token_of(link) {
        Some(token) => Answer::Ready { mode, port, link: one_time_link(link, port), token },
        None => Answer::Failed("Kora's panel gave a sign-in link the app cannot use. Run `kora panel --new-token`, then try again.".into()),
    }
}

/// The decision, with the real world behind two closures (tests give fakes).
pub(crate) fn answer(plan: Plan, windows: bool, running: Option<(u16, String)>, no_background: bool, start: &mut dyn FnMut() -> Result<(u16, String), String>) -> Answer {
    match plan {
        Plan::OpenRunning => match running {
            Some((port, link)) => ready("running", port, &link),
            None => Answer::Failed("Kora's panel stopped while it was being opened. Try again.".into()),
        },
        Plan::StartBackground if no_background => Answer::ForegroundNeeded { why: None },
        Plan::StartBackground => match start() {
            Ok((port, link)) => ready("started", port, &link),
            Err(why) => Answer::ForegroundNeeded { why: Some(format!("Kora could not set up the background panel ({why}).")) },
        },
        Plan::RunHere => Answer::ForegroundNeeded { why: None },
        Plan::RunAsUser(_) | Plan::StartAsNormalUser | Plan::ExplainElevated => Answer::Failed(bare::elevated_explanation(windows).join(" ")),
    }
}

pub(crate) fn run(cmd: DesktopCmd) -> i32 {
    match cmd {
        DesktopCmd::Session(a) => {
            let home = svc::home();
            let ports = SystemPorts;
            let platform = kora_core::service::Platform::current();
            let (facts, running) = bare::facts(&home, &ports, platform);
            let running = running.map(|r| {
                let (port, link) = r.port_and_link();
                (port, link.to_string())
            });
            let mut start = || bare::start_and_wait(&home, &ports);
            let out = answer(bare::decide(&facts), facts.windows, running, a.no_background, &mut start);
            // One line, last: anything printed while starting comes before it.
            println!("{}", out.to_json());
            i32::from(matches!(out, Answer::Failed(_)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK: &str = "http://127.0.0.1:47120/#token=tok123";

    fn no_start() -> impl FnMut() -> Result<(u16, String), String> {
        || panic!("must not start anything")
    }

    #[test]
    fn a_running_panel_is_handed_over_with_its_token() {
        let a = answer(Plan::OpenRunning, false, Some((47120, LINK.into())), false, &mut no_start());
        match a {
            // No panel answers in a unit test, so the link stays the token link (a real one has a one-time code).
            Answer::Ready { mode, port, token, link } => assert_eq!((mode, port, token.as_str(), link.as_str()), ("running", 47120, "tok123", LINK)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_stopped_panel_is_started_unless_told_not_to() {
        let mut started = 0;
        let a = answer(Plan::StartBackground, true, None, false, &mut || {
            started += 1;
            Ok((47121, "http://127.0.0.1:47121/#token=t2".into()))
        });
        assert!(matches!(&a, Answer::Ready { mode: "started", port: 47121, .. }), "{a:?}");
        assert_eq!(started, 1);
        assert_eq!(answer(Plan::StartBackground, true, None, true, &mut no_start()), Answer::ForegroundNeeded { why: None }, "--no-background starts nothing");
        let failed = answer(Plan::StartBackground, true, None, false, &mut || Err("no service manager".into()));
        assert!(matches!(&failed, Answer::ForegroundNeeded { why: Some(w) } if w.contains("no service manager")), "no dead end: the app runs the panel itself");
        assert_eq!(answer(Plan::RunHere, false, None, false, &mut no_start()), Answer::ForegroundNeeded { why: None });
    }

    #[test]
    fn administrator_sessions_get_the_explanation_and_never_a_panel() {
        for p in [Plan::ExplainElevated, Plan::StartAsNormalUser, Plan::RunAsUser("sam".into())] {
            let a = answer(p, true, None, false, &mut no_start());
            assert!(matches!(&a, Answer::Failed(m) if m.contains("administrator")), "{a:?}");
        }
    }

    #[test]
    fn the_json_is_one_line_the_app_can_read() {
        let a = Answer::Ready { mode: "running", port: 47120, link: "http://127.0.0.1:47120/#code=abcdefghijklmnop".into(), token: "tok".into() };
        let text = a.to_json().to_string();
        assert!(!text.contains('\n'));
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!((v["ok"].as_bool(), v["mode"].as_str(), v["port"].as_u64(), v["token"].as_str()), (Some(true), Some("running"), Some(47120), Some("tok")));
        assert_eq!(Answer::Failed("x".into()).to_json()["ok"], false);
        assert_eq!(Answer::ForegroundNeeded { why: None }.to_json()["mode"], "foreground_needed");
    }

    #[test]
    fn a_link_without_a_token_is_refused() {
        assert!(matches!(ready("running", 47120, "http://127.0.0.1:47120/"), Answer::Failed(_)));
    }
}
