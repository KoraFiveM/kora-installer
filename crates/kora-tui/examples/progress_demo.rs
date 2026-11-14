//! Manual QA: feed fake progress events, then show the outcome screen.
use kora_core::installer::{InstallOutcome, ProgressEvent, StepStatus};
use std::{sync::mpsc, thread, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let steps = ["Check tools", "Download server software", "Download Kora resources", "Create database", "Write settings", "Start server"];
        for (i, t) in steps.iter().enumerate() {
            let id = format!("demo.{i}");
            let pct = (i * 100 / steps.len()) as u8;
            let ev = |status, detail: String, percent| ProgressEvent { step: id.clone(), title: t.to_string(), status, detail, percent };
            let _ = tx.send(ev(StepStatus::Running, format!("{t}..."), pct));
            for n in 1..=5 {
                thread::sleep(Duration::from_millis(250));
                let _ = tx.send(ev(StepStatus::Running, format!("{t} ({n}/5)"), pct + (n * 100 / steps.len() / 5) as u8));
            }
            let _ = tx.send(ev(StepStatus::Done, format!("{t} done"), ((i + 1) * 100 / steps.len()) as u8));
        }
    });
    kora_tui::run_install_progress(rx)?;
    kora_tui::show_outcome(&InstallOutcome {
        dir: "/tmp/kora-demo".into(),
        connect: "connect localhost:30120".into(),
        txadmin_url: "http://localhost:40120".into(),
        notes: vec!["This was a demo; nothing was installed.".into()],
    })
}
