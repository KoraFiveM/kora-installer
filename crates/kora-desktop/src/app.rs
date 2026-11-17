//! The app: one event loop, the tray, at most one window, and the glue between them. Every decision comes from
//! `kora-shell`; this file only turns those decisions into windows, menu items and operating-system calls.

use std::path::PathBuf;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kora_shell::bridge::{self, BridgeError, Command as BridgeCommand, Reply};
use kora_shell::instance::{self, Message};
use kora_shell::model::{AlertRow, ServerRow, ServerStatus, Snapshot};
use kora_shell::nav::{self, Nav};
use kora_shell::platform;
use kora_shell::settings::Settings;
use kora_shell::toast::{Notifier, Policy, Toast};
use kora_shell::tray::{self, Action, MenuOptions};
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Window, WindowBuilder};
use wry::{NewWindowResponse, PageLoadEvent, WebContext, WebView, WebViewBuilder};

use crate::net::{self, Creds};
use crate::smoke::Smoke;
use crate::tray_ui::{self, Tray};
use crate::{os, toasts};

pub type Target = EventLoopWindowTarget<UserEvent>;

/// Everything that can happen, delivered to the event loop.
pub enum UserEvent {
    /// A tray menu item was chosen (its id).
    Menu(String),
    /// A click on the tray icon itself.
    TrayOpen,
    /// A signed-in panel: its port, a one-time sign-in link and how to call its API.
    Session {
        port: u16,
        link: String,
        creds: Creds,
    },
    SessionFailed(String),
    Connected(bool),
    Servers(Vec<ServerRow>),
    Alerts(Vec<AlertRow>),
    Update(Option<String>),
    /// A second launch asked us to show the window.
    Instance(Message),
    /// A notification was clicked, or the page is to be shown on this route.
    OpenRoute(String),
    /// A message from the page (the bridge). `trusted` = it came from the panel's own origin.
    Bridge {
        raw: String,
        trusted: bool,
    },
    /// The answer to a bridge request that needed a dialog.
    Reply(Reply),
    /// A notification the app wants shown (errors of tray actions, update results).
    Toast(Toast),
    PageLoaded(String),
    /// `document.title` of the page (smoke test).
    SmokeTitle(String),
}

struct Win {
    window: Window,
    webview: WebView,
}

pub struct Options {
    pub background: bool,
    pub route: Option<String>,
    pub kora: PathBuf,
    pub exe: PathBuf,
    pub state_dir: PathBuf,
    pub smoke: Option<PathBuf>,
}

pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    opts: Options,
    settings: Settings,
    notifier: Notifier,
    snapshot: Snapshot,
    creds: Option<Creds>,
    link: Option<String>,
    port: u16,
    win: Option<Win>,
    web_context: Option<WebContext>,
    tray: Option<Tray>,
    autostart: bool,
    want_open: Option<String>,
    was_paused: bool,
    warned_webview: bool,
    last_tick: Instant,
    pub foreground_panel: Arc<Mutex<Option<Child>>>,
    smoke: Option<Smoke>,
    instance: Option<instance::First>,
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>, opts: Options, instance: instance::First, foreground_panel: Arc<Mutex<Option<Child>>>) -> App {
        let settings = Settings::load(&opts.state_dir);
        let mut notifier = Notifier::new(Policy { quiet: settings.quiet(), ..Policy::default() });
        notifier.pause_until(settings.pause_until.filter(|u| *u > unix_now()));
        let smoke = opts.smoke.clone().map(Smoke::new);
        App {
            proxy,
            settings,
            notifier,
            snapshot: Snapshot::default(),
            creds: None,
            link: None,
            port: 0,
            win: None,
            web_context: None,
            tray: None,
            autostart: false,
            want_open: None,
            was_paused: false,
            warned_webview: false,
            last_tick: Instant::now(),
            foreground_panel,
            smoke,
            instance: Some(instance),
            opts,
        }
    }

    /// The thread that tells the loop about second launches.
    pub fn spawn_instance_listener(&mut self) {
        let Some(first) = self.instance.take() else { return };
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let Ok(listener) = first.listener.try_clone() else { return };
            for stream in listener.incoming().flatten() {
                if let Some(m) = first.handle(stream) {
                    if proxy.send_event(UserEvent::Instance(m)).is_err() {
                        break;
                    }
                }
            }
        });
    }

    fn menu_options(&self) -> MenuOptions {
        MenuOptions { autostart: self.autostart, paused: self.notifier.paused(unix_now()), windows: cfg!(windows), max_servers: 12 }
    }

    fn refresh_tray(&mut self) {
        let entries = tray::build_menu(&self.snapshot, &self.menu_options());
        let kind = tray::health(&self.snapshot).icon();
        let tip = tray::tooltip(&self.snapshot);
        if let Some(s) = &mut self.smoke {
            s.menu_ids = tray::ids(&entries);
            s.health = format!("{:?}", tray::health(&self.snapshot));
        }
        if let Some(t) = &mut self.tray {
            t.update(kind, &tip, entries);
        }
    }

    fn say(&self, title: &str, body: &str, route: &str) {
        toasts::show(Toast { title: title.into(), body: body.into(), route: route.into(), urgent: false }, self.proxy.clone());
    }

    // ---- the window --------------------------------------------------------------------------------------------

    fn open_window(&mut self, target: &Target, route: &str) {
        let route = nav::route_or_home(Some(route));
        let Some(creds) = self.creds.clone() else {
            crate::smoke::note("the window waits for the panel's session");
            self.want_open = Some(route);
            self.say("Kora is starting", "The window opens as soon as Kora's panel is ready.", "/");
            return;
        };
        if let Some(w) = &self.win {
            w.window.set_visible(true);
            w.window.set_minimized(false);
            w.window.set_focus();
            if route != "/" {
                let _ = w.webview.load_url(&nav::panel_url(self.port, &route));
            }
            return;
        }
        // A new window: a fresh one-time code (the first one was for the first load and expires).
        let link = net::fresh_link(&creds).ok().or_else(|| self.link.clone());
        let Some(link) = link else {
            self.say("Kora could not open its window", "Kora's panel did not give a sign-in link. Try again in a moment.", "/");
            return;
        };
        let url = kora_shell::session::link_on_route(&link, &route);
        if let Err(why) = os::webview_version() {
            self.fall_back_to_browser(&url, &why);
            return;
        }
        match self.create_window(target, &url) {
            Ok(w) => self.win = Some(w),
            Err(why) => self.fall_back_to_browser(&url, &why),
        }
    }

    /// No web view: open the browser instead (everything works there) and say why, once.
    fn fall_back_to_browser(&mut self, url: &str, why: &str) {
        crate::smoke::note(format!("no window ({why}); opening the browser instead"));
        os::open_in_browser(url);
        if !self.warned_webview {
            self.warned_webview = true;
            let lines = platform::webview_missing_message(cfg!(windows));
            self.say("Kora opened in your browser", &lines.join(" "), "/");
        }
    }

    fn create_window(&mut self, target: &Target, url: &str) -> Result<Win, String> {
        let (w, h) = self.settings.window_size();
        let mut wb = WindowBuilder::new().with_title(platform::window_title("")).with_inner_size(LogicalSize::new(w as f64, h as f64)).with_visible(true);
        if let Some(i) = tray_ui::window_icon() {
            wb = wb.with_window_icon(Some(i));
        }
        let window = wb.build(target).map_err(|e| e.to_string())?;
        let port = self.port;
        let (ipc_proxy, load_proxy) = (self.proxy.clone(), self.proxy.clone());
        let ctx = self.web_context.get_or_insert_with(|| WebContext::new(Some(self.opts.state_dir.join("webview"))));
        let builder = WebViewBuilder::new_with_web_context(ctx)
            .with_url(url)
            // Only the page's main frame gets `window.koraShell`.
            .with_initialization_script_for_main_only(bridge::INIT_SCRIPT, true)
            .with_navigation_handler(move |u| match nav::decide(&u, port) {
                Nav::Allow => true,
                Nav::External(external) => {
                    os::open_in_browser(&external);
                    false
                }
                Nav::Deny => false,
            })
            .with_new_window_req_handler(move |u, _features| {
                // A link that wants a new window: web links go to the browser, nothing opens inside the app.
                if let Nav::External(external) = nav::decide(&u, port) {
                    os::open_in_browser(&external);
                }
                NewWindowResponse::Deny
            })
            .with_ipc_handler(move |req| {
                let trusted = nav::trusted_page(&req.uri().to_string(), port);
                let _ = ipc_proxy.send_event(UserEvent::Bridge { raw: req.body().clone(), trusted });
            })
            .with_on_page_load_handler(move |event, url| {
                if matches!(event, PageLoadEvent::Finished) {
                    let _ = load_proxy.send_event(UserEvent::PageLoaded(url));
                }
            });
        #[cfg(windows)]
        let webview = builder.build(&window).map_err(|e| e.to_string())?;
        #[cfg(not(windows))]
        let webview = {
            use tao::platform::unix::WindowExtUnix;
            use wry::WebViewBuilderExtUnix;
            let vbox = window.default_vbox().ok_or("the window has no container")?;
            builder.build_gtk(vbox).map_err(|e| e.to_string())?
        };
        Ok(Win { window, webview })
    }

    #[cfg(windows)]
    fn hwnd(&self) -> isize {
        use tao::platform::windows::WindowExtWindows;
        self.win.as_ref().map(|w| w.window.hwnd()).unwrap_or(0)
    }

    #[cfg(not(windows))]
    fn hwnd(&self) -> isize {
        0
    }

    fn reply(&self, r: Reply) {
        if let Some(w) = &self.win {
            let _ = w.webview.evaluate_script(&r.script());
        }
    }

    // ---- the page's requests (the bridge) ---------------------------------------------------------------------

    fn on_bridge(&mut self, raw: &str, trusted: bool) {
        let req = match bridge::parse(raw, trusted) {
            Ok(r) => r,
            Err(BridgeError::Refused(Some(id), why)) => return self.reply(Reply::err(id, why)),
            Err(_) => return,
        };
        let id = req.id;
        match req.command {
            BridgeCommand::Hello => self.reply(Reply::ok(id, bridge::hello_value(env!("CARGO_PKG_VERSION"), cfg!(windows), cfg!(windows)))),
            BridgeCommand::OpenExternal { url } => {
                os::open_in_browser(&url);
                self.reply(Reply::ok(id, serde_json::Value::Null));
            }
            BridgeCommand::Notify { title, body, route, urgent } => {
                self.notifier.on_page_event(unix_now(), Toast { title, body, route: route.unwrap_or_else(|| "/".into()), urgent });
                self.reply(Reply::ok(id, serde_json::Value::Null));
            }
            BridgeCommand::PickFolder { title } => {
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let mut d = rfd::FileDialog::new();
                    if let Some(t) = title {
                        d = d.set_title(t);
                    }
                    let _ = proxy.send_event(UserEvent::Reply(path_reply(id, d.pick_folder())));
                });
            }
            BridgeCommand::PickFile { title, extensions } => {
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let mut d = rfd::FileDialog::new();
                    if let Some(t) = title {
                        d = d.set_title(t);
                    }
                    if !extensions.is_empty() {
                        d = d.add_filter("Files", &extensions);
                    }
                    let _ = proxy.send_event(UserEvent::Reply(path_reply(id, d.pick_file())));
                });
            }
            BridgeCommand::Elevate { action } => {
                let command = platform::elevated_command(action, &self.opts.kora);
                if let Some(w) = &self.win {
                    w.window.set_visible(true);
                    w.window.set_focus();
                }
                let r = match os::elevate(self.hwnd(), &command) {
                    Ok(()) => Reply::ok(id, serde_json::json!({ "started": true })),
                    Err(why) => Reply::err(id, why),
                };
                self.reply(r);
            }
        }
    }

    // ---- the tray's actions -----------------------------------------------------------------------------------

    fn on_action(&mut self, target: &Target, action: Action, flow: &mut ControlFlow) {
        match action {
            Action::Open(route) => self.open_window(target, &route),
            Action::Control { server, verb } => {
                let Some(c) = self.creds.clone() else { return };
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    if let Err(why) = net::control(&c, &server, verb) {
                        let t = Toast { title: format!("Could not {} {server}", verb.api_word()), body: why, route: nav::server_route(&server), urgent: false };
                        let _ = proxy.send_event(UserEvent::Toast(t));
                    }
                });
            }
            Action::OpenTxAdmin { server } => {
                if let Some(v) = self.snapshot.servers.iter().find(|v| v.name == server && v.status == ServerStatus::Running) {
                    // The shell runs on the same computer as Kora: txAdmin is on localhost.
                    os::open_in_browser(&format!("http://localhost:{}", v.ports.txadmin));
                }
            }
            Action::CheckUpdates => {
                let Some(c) = self.creds.clone() else { return };
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let t = match net::self_update(&c) {
                        Ok(Some(v)) => {
                            let _ = proxy.send_event(UserEvent::Update(Some(v.clone())));
                            Toast { title: format!("Kora {v} is ready"), body: "Open Kora to update. Your servers keep running while it does.".into(), route: "/settings".into(), urgent: false }
                        }
                        Ok(None) => Toast { title: "Kora is up to date".into(), body: "You have the newest version.".into(), route: "/settings".into(), urgent: false },
                        Err(why) => Toast { title: "Could not check for updates".into(), body: why, route: "/settings".into(), urgent: false },
                    };
                    let _ = proxy.send_event(UserEvent::Toast(t));
                });
            }
            Action::PauseNotifications(minutes) => {
                let until = (minutes > 0).then(|| unix_now() + u64::from(minutes) * 60);
                self.notifier.pause_until(until);
                self.settings.pause_until = until;
                let _ = self.settings.save(&self.opts.state_dir);
                self.refresh_tray();
            }
            Action::ToggleAutostart => {
                let want = !self.autostart;
                match os::set_autostart(want, &self.opts.exe) {
                    Ok(()) => self.autostart = os::autostart_enabled(&self.opts.exe),
                    Err(why) => self.say("Could not change start-up", &why, "/"),
                }
                self.refresh_tray();
            }
            Action::Quit => self.quit(flow),
            Action::QuitAndStopService => {
                // The background service goes; the servers themselves keep running.
                let _ = os::run_quiet(&platform::stop_service_command(&self.opts.kora));
                if let Some(mut child) = self.foreground_panel.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    let _ = child.kill();
                }
                self.quit(flow);
            }
        }
    }

    fn quit(&mut self, flow: &mut ControlFlow) {
        // Drop the tray first: Windows leaves a ghost icon behind otherwise.
        self.tray = None;
        self.win = None;
        *flow = ControlFlow::Exit;
    }

    fn close_to_tray(&mut self, flow: &mut ControlFlow) {
        if let Some(w) = &self.win {
            let scale = w.window.scale_factor();
            let size = w.window.inner_size().to_logical::<f64>(scale);
            self.settings.window_width = Some(size.width.round() as u32);
            self.settings.window_height = Some(size.height.round() as u32);
        }
        if self.tray.is_none() {
            // No tray here (a desktop without a notification area): closing the window is quitting.
            return self.quit(flow);
        }
        if let Some(w) = &self.win {
            w.window.set_visible(false);
        }
        if !self.settings.close_hint_shown {
            self.settings.close_hint_shown = true;
            let how = if cfg!(windows) {
                "Kora is still running in the notification area (the little arrow by the clock). Right-click its icon for options, including \"Start with Windows\"."
            } else {
                "Kora is still running in your system tray. Use its menu to quit."
            };
            self.say("Kora is still running", how, "/");
        }
        let _ = self.settings.save(&self.opts.state_dir);
    }

    // ---- ticks and the smoke test -----------------------------------------------------------------------------

    fn tick(&mut self, target: &Target, flow: &mut ControlFlow) {
        self.last_tick = Instant::now();
        let now = unix_now();
        let minute = if self.settings.quiet().is_some() { os::local_minute_of_day() } else { 0 };
        for t in self.notifier.drain(now, minute) {
            toasts::show(t, self.proxy.clone());
        }
        let paused = self.notifier.paused(now);
        if paused != self.was_paused {
            self.was_paused = paused;
            self.refresh_tray();
        }
        self.smoke_tick(target, flow);
    }

    fn smoke_tick(&mut self, _target: &Target, flow: &mut ControlFlow) {
        let connected = self.snapshot.connected;
        let url = self.win.as_ref().and_then(|w| w.webview.url().ok());
        let title = self.win.as_ref().map(|w| w.window.title());
        let tray_built = self.tray.is_some();
        let Some(s) = &mut self.smoke else { return };
        if s.timed_out() {
            s.write(false, "timed out waiting for the window and the panel", url.as_deref(), title.as_deref(), connected, tray_built);
            *flow = ControlFlow::ExitWithCode(1);
            return;
        }
        if s.should_ask_title(connected) {
            if let Some(w) = &self.win {
                let proxy = Mutex::new(self.proxy.clone());
                let _ = w.webview.evaluate_script_with_callback("document.title", move |t| {
                    if let Ok(p) = proxy.lock() {
                        let _ = p.send_event(UserEvent::SmokeTitle(t));
                    }
                });
            }
        }
        if s.finished() {
            s.write(true, "", url.as_deref(), title.as_deref(), connected, tray_built);
            *flow = ControlFlow::Exit;
        }
    }

    // ---- the event loop ---------------------------------------------------------------------------------------

    pub fn handle(&mut self, event: Event<UserEvent>, target: &Target, flow: &mut ControlFlow) {
        *flow = ControlFlow::WaitUntil(self.last_tick + Duration::from_secs(1));
        match event {
            Event::NewEvents(StartCause::Init) => self.on_start(target),
            Event::MainEventsCleared => {
                if self.last_tick.elapsed() >= Duration::from_secs(1) {
                    self.tick(target, flow);
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => self.close_to_tray(flow),
            Event::UserEvent(e) => self.on_user(e, target, flow),
            _ => {}
        }
    }

    fn on_start(&mut self, target: &Target) {
        let entries = tray::build_menu(&self.snapshot, &self.menu_options());
        match Tray::new(tray::health(&self.snapshot).icon(), &tray::tooltip(&self.snapshot), entries) {
            Ok(t) => self.tray = Some(t),
            Err(why) => eprintln!("kora-desktop: no tray icon ({why}); running with a window only"),
        }
        if self.smoke.is_none() {
            os::register_toast_identity(None);
            self.autostart = os::autostart_enabled(&self.opts.exe);
            if !self.settings.autostart_default_applied {
                // The first run turns "Start with Windows" on, once; turning it off in the tray menu sticks.
                if os::set_autostart(true, &self.opts.exe).is_ok() {
                    self.autostart = os::autostart_enabled(&self.opts.exe);
                }
                self.settings.autostart_default_applied = true;
                let _ = self.settings.save(&self.opts.state_dir);
            }
        }
        self.refresh_tray();
        if !self.opts.background || self.smoke.is_some() || self.tray.is_none() {
            self.want_open = Some(nav::route_or_home(self.opts.route.as_deref()));
        }
        let _ = target;
    }

    fn on_user(&mut self, e: UserEvent, target: &Target, flow: &mut ControlFlow) {
        match e {
            UserEvent::Menu(id) => {
                if let Some(a) = tray::decode(&id) {
                    self.on_action(target, a, flow);
                }
            }
            UserEvent::TrayOpen => self.open_window(target, "/"),
            UserEvent::Session { port, link, creds } => {
                let restarted = self.creds.is_some();
                self.port = port;
                self.link = Some(link);
                self.creds = Some(creds);
                if let Some(route) = self.want_open.take() {
                    self.open_window(target, &route);
                } else if restarted {
                    // The panel was restarted: the page's sign-in may be gone. Load it again with a new code.
                    if let (Some(w), Some(c)) = (&self.win, &self.creds) {
                        if let Ok(link) = net::fresh_link(c) {
                            let _ = w.webview.load_url(&link);
                        }
                    }
                }
            }
            UserEvent::SessionFailed(why) => {
                crate::smoke::note(format!("session failed (shown to the user): {why}"));
                if self.want_open.take().is_some() {
                    self.say("Kora could not start", &why, "/");
                }
                self.snapshot.connected = false;
                self.refresh_tray();
            }
            UserEvent::Connected(c) => {
                self.snapshot.connected = c;
                if !c {
                    self.snapshot.servers.clear();
                }
                self.refresh_tray();
            }
            UserEvent::Servers(v) => {
                self.snapshot.servers = v;
                self.refresh_tray();
            }
            UserEvent::Alerts(a) => {
                self.notifier.on_alerts(unix_now(), &a);
                self.snapshot.alerts = a;
                self.refresh_tray();
            }
            UserEvent::Update(u) => {
                self.notifier.on_update(unix_now(), u.as_deref());
                self.snapshot.update = u;
                self.refresh_tray();
            }
            UserEvent::Instance(Message::Focus(route)) => {
                self.open_window(target, &nav::route_or_home(route.as_deref()));
                if let Some(s) = &mut self.smoke {
                    s.second_launch = Some(route.unwrap_or_default());
                }
            }
            UserEvent::OpenRoute(route) => self.open_window(target, &route),
            UserEvent::Bridge { raw, trusted } => self.on_bridge(&raw, trusted),
            UserEvent::Reply(r) => self.reply(r),
            UserEvent::Toast(t) => toasts::show(t, self.proxy.clone()),
            UserEvent::PageLoaded(url) => {
                crate::smoke::note(format!("page loaded: {}", url.split(['#', '?']).next().unwrap_or("")));
                if let Some(s) = &mut self.smoke {
                    s.page_loaded(&url);
                }
            }
            UserEvent::SmokeTitle(raw) => {
                let title = serde_json::from_str::<String>(&raw).unwrap_or(raw);
                let drive = self.smoke.as_mut().map(|s| s.got_title(title)).unwrap_or(false);
                if drive {
                    // Drive one tray action: "Open Kora" on the settings page, exactly as the menu would.
                    if let Some(a) = tray::decode(&tray::encode(&Action::Open("/settings".into()))) {
                        self.on_action(target, a, flow);
                    }
                }
            }
        }
    }
}

fn path_reply(id: u32, p: Option<PathBuf>) -> Reply {
    Reply::ok(id, p.map(|p| serde_json::Value::String(p.to_string_lossy().into_owned())).unwrap_or(serde_json::Value::Null))
}
