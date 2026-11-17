//! Kora desktop app: a tray icon and a native window that shows Kora Panel.
//!
//! A thin client of the panel (`kora panel`, loopback). It never serves anything, never talks to the internet
//! (except to hand a link to your browser) and holds no Kora data. All decisions are in the `kora-shell` crate
//! (tested everywhere); this crate is the toolkit glue. See docs/desktop-app.md.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod net;
mod os;
mod smoke;
mod toasts;
mod tray_ui;

use std::sync::{Arc, Mutex};

use kora_shell::instance::{self, Acquired, Message};
use kora_shell::{cli, platform, settings};
use tao::event_loop::EventLoopBuilder;
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

use crate::app::{App, Options, UserEvent};

fn main() {
    os::attach_console();
    let args = match cli::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if args.help {
        print!("{}", cli::HELP);
        return;
    }
    if args.version {
        println!("kora-desktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    if args.uninstall {
        let _ = os::set_autostart(false, &exe);
        os::unregister_toast_identity();
        println!("The Kora desktop app no longer starts when you sign in.");
        return;
    }
    let Some(state_dir) = settings::state_dir(cfg!(windows), &os::env) else {
        eprintln!("Kora could not find your user folder.");
        std::process::exit(1);
    };

    // One at a time: a second launch asks the first to show its window.
    let first = match instance::acquire(&state_dir, &Message::Focus(args.route.clone())) {
        Ok(Acquired::First(f)) => f,
        Ok(Acquired::Existing { told }) => {
            if !told {
                eprintln!("Kora is already running but did not answer.");
            }
            return;
        }
        Err(e) => {
            eprintln!("Kora could not start ({e}).");
            std::process::exit(1);
        }
    };

    let kora = args.kora.clone().or_else(|| platform::kora_beside(&exe, cfg!(windows), &|p| p.is_file())).unwrap_or_else(|| std::path::PathBuf::from(if cfg!(windows) { "kora.exe" } else { "kora" }));
    let smoke = args.smoke.clone();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // The tray's own events arrive on other threads: forward them into the loop.
    let shared = Arc::new(Mutex::new(proxy.clone()));
    let menu_proxy = shared.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if let Ok(p) = menu_proxy.lock() {
            let _ = p.send_event(UserEvent::Menu(e.id.0.clone()));
        }
    }));
    let click_proxy = shared.clone();
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            if let Ok(p) = click_proxy.lock() {
                let _ = p.send_event(UserEvent::TrayOpen);
            }
        }
    }));

    let foreground = Arc::new(Mutex::new(None));
    // The smoke test must leave no background service behind.
    net::spawn(proxy.clone(), kora.clone(), smoke.is_some(), foreground.clone());

    let mut app = App::new(proxy, Options { background: args.background, route: args.route, kora, exe, state_dir, smoke }, first, foreground);
    app.spawn_instance_listener();
    event_loop.run(move |event, target, flow| app.handle(event, target, flow));
}
