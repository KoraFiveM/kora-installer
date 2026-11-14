use super::*;
use crate::wizard::{PortEnv, WizardState};
use crossterm::event::{KeyEventKind, KeyEventState};
use kora_core::firewall::ports::PortProber;
use kora_core::installer::Mode;
use ratatui::{backend::TestBackend, Terminal};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

// ---- fakes ----

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn owner(login: &str, kind: OwnerKind, can: bool, reason: Option<&str>) -> Owner {
    Owner { login: login.into(), kind, can_fork: can, reason: reason.map(String::from), role: None }
}

fn sample() -> Vec<Owner> {
    vec![
        owner("team-a", OwnerKind::Org, true, None),
        owner("octo", OwnerKind::User, true, None),
        owner("corp-sso", OwnerKind::Org, false, Some("This organisation requires SAML single sign-on. Authorise Kora for it on GitHub first.")),
        owner("team-b", OwnerKind::Org, true, None),
    ]
}

struct Fake {
    auth: Mutex<GhAuth>,
    owners: Mutex<Result<Vec<Owner>, String>>,
    login_tx: Mutex<Option<mpsc::Sender<Result<GhAuth, String>>>>,
    login_cancelled: Arc<AtomicBool>,
    watch_tx: Mutex<Option<mpsc::Sender<Option<String>>>>,
    watch_stop: Mutex<Option<Arc<AtomicBool>>>,
    watch_before: Mutex<Vec<String>>,
    owner_calls: Mutex<usize>,
}

impl Fake {
    fn new(auth: GhAuth, owners: Result<Vec<Owner>, String>) -> Arc<Self> {
        Arc::new(Self {
            auth: Mutex::new(auth),
            owners: Mutex::new(owners),
            login_tx: Mutex::new(None),
            login_cancelled: Arc::new(AtomicBool::new(false)),
            watch_tx: Mutex::new(None),
            watch_stop: Mutex::new(None),
            watch_before: Mutex::new(vec![]),
            owner_calls: Mutex::new(0),
        })
    }
}

impl GithubApi for Fake {
    fn auth(&self) -> GhAuth {
        lock(&self.auth).clone()
    }
    fn list_owners(&self) -> Result<Vec<Owner>, String> {
        *lock(&self.owner_calls) += 1;
        lock(&self.owners).clone()
    }
    fn begin_login(&self) -> Result<LoginHandle, String> {
        let (tx, rx) = mpsc::channel();
        *lock(&self.login_tx) = Some(tx);
        let flag = self.login_cancelled.clone();
        Ok(LoginHandle {
            code: "ABCD-1234".into(),
            url: "https://github.com/login/device".into(),
            done: rx,
            cancel: Box::new(move || flag.store(true, Ordering::SeqCst)),
        })
    }
    fn org_creation_url(&self) -> String {
        "https://github.com/account/organizations/new?plan=free".into()
    }
    fn watch_new_org(&self, before: Vec<String>) -> OrgWatch {
        let (tx, rx) = mpsc::channel();
        *lock(&self.watch_tx) = Some(tx);
        *lock(&self.watch_before) = before;
        let stop = Arc::new(AtomicBool::new(false));
        *lock(&self.watch_stop) = Some(stop.clone());
        OrgWatch { found: rx, stop }
    }
}

#[derive(Clone, Default)]
struct Opened(Arc<Mutex<Vec<String>>>);
impl UrlOpener for Opened {
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        lock(&self.0).push(url.into());
        Ok(())
    }
}

#[derive(Clone, Default)]
struct Clip(Arc<Mutex<String>>);
impl Clipboard for Clip {
    fn get_text(&mut self) -> Result<String, String> {
        Ok(lock(&self.0).clone())
    }
    fn set_text(&mut self, t: &str) -> Result<(), String> {
        *lock(&self.0) = t.into();
        Ok(())
    }
}

struct Setup {
    api: Arc<Fake>,
    opened: Opened,
    clip: Clip,
    dir: tempfile::TempDir,
}

impl Setup {
    fn new(auth: GhAuth, owners: Result<Vec<Owner>, String>) -> Self {
        Self { api: Fake::new(auth, owners), opened: Opened::default(), clip: Clip::default(), dir: tempfile::tempdir().unwrap() }
    }
    fn store(&self) -> PathBuf {
        self.dir.path().join("github-owner.txt")
    }
    fn flow(&self) -> GithubFlow {
        GithubFlow::new(self.api.clone())
            .with_opener(Box::new(self.opened.clone()))
            .with_clipboard(Box::new(self.clip.clone()))
            .with_store(Some(self.store()))
    }
}

fn logged_in() -> GhAuth {
    GhAuth::LoggedIn { login: "octo".into() }
}

fn until(f: &mut GithubFlow, what: &str, cond: impl Fn(&GithubFlow) -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !cond(f) {
        assert!(Instant::now() < end, "timed out waiting for {what}; phase = {:?}", f.phase);
        f.tick();
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn settle(f: &mut GithubFlow) {
    until(f, "background work", |f| !f.busy());
}

fn key(c: KeyCode) -> KeyEvent {
    KeyEvent { code: c, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
}

fn typed(f: &mut GithubFlow, s: &str) {
    for c in s.chars() {
        f.key(key(KeyCode::Char(c)));
    }
}

fn click(x: u16, y: u16) -> MouseEvent {
    MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE }
}

const REGION: Rect = Rect { x: 0, y: 0, width: 80, height: 20 };

fn click_btn(f: &mut GithubFlow, b: Btn) -> Nav {
    let r = f.layout(REGION).buttons.iter().find(|(x, _)| *x == b).map(|(_, r)| *r).unwrap_or_else(|| panic!("no {b:?} button in {:?}", f.phase));
    f.click(click(r.x + 1, r.y), REGION)
}

fn draw(f: &GithubFlow, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|fr| render(fr, f, Rect::new(0, 0, w, h))).unwrap();
    text_of(&t)
}

fn text_of(t: &Terminal<TestBackend>) -> String {
    let b = t.backend().buffer();
    let mut out = String::new();
    for y in 0..b.area.height {
        for x in 0..b.area.width {
            out.push_str(b[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn ready_picker(s: &Setup) -> GithubFlow {
    let mut f = s.flow();
    f.enter("");
    settle(&mut f);
    assert_eq!(f.phase, Phase::Picker);
    f
}

// ---- sign-in ----

#[test]
fn logged_out_shows_code_and_waits() {
    let s = Setup::new(GhAuth::LoggedOut, Ok(sample()));
    let mut f = s.flow();
    f.enter("");
    until(&mut f, "the code", |f| matches!(f.phase, Phase::SignIn { .. }));
    let t = draw(&f, 80, 16);
    assert!(t.contains("ABCD-1234"), "{t}");
    assert!(t.contains("Waiting for you to approve in the browser..."), "{t}");
    for b in ["[ Copy code ]", "[ Open GitHub ]", "[ Cancel ]"] {
        assert!(t.contains(b), "{b}: {t}");
    }
    assert!(t.contains("https://github.com/login/device"), "{t}");
}

#[test]
fn copy_and_open_buttons_work_by_mouse_and_key() {
    let s = Setup::new(GhAuth::LoggedOut, Ok(sample()));
    let mut f = s.flow();
    f.enter("");
    until(&mut f, "the code", |f| matches!(f.phase, Phase::SignIn { .. }));
    click_btn(&mut f, Btn::Copy);
    assert_eq!(*lock(&s.clip.0), "ABCD-1234");
    assert!(f.status.as_deref().unwrap().contains("copied"));
    *lock(&s.clip.0) = String::new();
    f.key(key(KeyCode::Char('c')));
    assert_eq!(*lock(&s.clip.0), "ABCD-1234");
    click_btn(&mut f, Btn::OpenGithub);
    f.key(key(KeyCode::Char('o')));
    assert_eq!(*lock(&s.opened.0), vec!["https://github.com/login/device".to_string(); 2]);
}

#[test]
fn approving_signs_in_and_lists_accounts() {
    let s = Setup::new(GhAuth::LoggedOut, Ok(sample()));
    let mut f = s.flow();
    f.enter("");
    until(&mut f, "the code", |f| matches!(f.phase, Phase::SignIn { .. }));
    lock(&s.api.login_tx).as_ref().unwrap().send(Ok(logged_in())).unwrap();
    until(&mut f, "the picker", |f| f.phase == Phase::Picker);
    assert_eq!(f.signed_in.as_deref(), Some("octo"));
    assert!(draw_with_lines(&f).contains("Signed in as octo"));
}

/// The generic frame draws `lines()`; the tests draw them together with the body.
fn draw_with_lines(f: &GithubFlow) -> String {
    format!("{}\n{}", f.lines().join("\n"), draw(f, 80, 16))
}

#[test]
fn cancelling_the_sign_in_stops_it_and_offers_typing() {
    let s = Setup::new(GhAuth::LoggedOut, Ok(sample()));
    let mut f = s.flow();
    f.enter("");
    until(&mut f, "the code", |f| matches!(f.phase, Phase::SignIn { .. }));
    click_btn(&mut f, Btn::Cancel);
    assert!(s.api.login_cancelled.load(Ordering::SeqCst), "the gh login was cancelled");
    assert_eq!(f.phase, Phase::Manual);
    assert!(draw_with_lines(&f).contains("Sign-in cancelled"));
    assert!(!f.busy());
}

#[test]
fn escape_cancels_the_sign_in_too() {
    let s = Setup::new(GhAuth::LoggedOut, Ok(sample()));
    let mut f = s.flow();
    f.enter("");
    until(&mut f, "the code", |f| matches!(f.phase, Phase::SignIn { .. }));
    assert_eq!(f.key(key(KeyCode::Esc)), Nav::Stay);
    assert_eq!(f.phase, Phase::Manual);
}

#[test]
fn a_failed_sign_in_says_so_and_can_be_retried() {
    let s = Setup::new(GhAuth::LoggedOut, Ok(sample()));
    let mut f = s.flow();
    f.enter("");
    until(&mut f, "the code", |f| matches!(f.phase, Phase::SignIn { .. }));
    lock(&s.api.login_tx).as_ref().unwrap().send(Err("the code expired".into())).unwrap();
    until(&mut f, "manual", |f| f.phase == Phase::Manual);
    assert!(f.lines().join(" ").contains("the code expired"));
    click_btn(&mut f, Btn::Retry);
    until(&mut f, "a new code", |f| matches!(f.phase, Phase::SignIn { .. }));
}

#[test]
fn missing_cli_explains_and_falls_back_to_typing() {
    let s = Setup::new(GhAuth::Missing, Ok(sample()));
    let mut f = s.flow();
    f.enter("");
    settle(&mut f);
    assert_eq!(f.phase, Phase::Manual);
    let lines = f.lines().join(" ");
    assert!(lines.contains("installs it during setup") && lines.contains("next time"), "{lines}");
    assert_eq!(f.chosen(), None);
    typed(&mut f, "my-team");
    assert_eq!(f.chosen().as_deref(), Some("my-team"));
}

// ---- owner list ----

#[test]
fn personal_account_comes_first_and_is_preselected() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let f = ready_picker(&s);
    assert_eq!(f.owners[0].login, "octo");
    assert_eq!(f.chosen().as_deref(), Some("octo"));
    let t = draw_with_lines(&f);
    assert!(t.contains("Where should your copies (forks) of every Kora script go?") || f.title().contains("Where should your copies"));
    assert!(t.contains("An organisation keeps all ~55 copies together and lets your team share them; your own account works too."));
    let user = t.find("octo (your account)").expect("user row");
    let org = t.find("team-a (organisation)").expect("org row");
    assert!(user < org, "personal account is listed first");
    for b in ["[ Refresh ]", "[ Create a new organisation... ]"] {
        assert!(t.contains(b), "{b}");
    }
}

#[test]
fn unusable_entries_are_greyed_with_the_reason_and_cannot_be_selected() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut f = ready_picker(&s);
    let mut term = Terminal::new(TestBackend::new(100, 16)).unwrap();
    term.draw(|fr| render(fr, &f, Rect::new(0, 0, 100, 16))).unwrap();
    let t = text_of(&term);
    assert!(t.contains("corp-sso (organisation)") && t.contains("requires SAML single sign-on"), "{t}");
    // The row is drawn in dark grey.
    let l = f.layout(Rect::new(0, 0, 100, 16));
    let (_, r) = l.entries.iter().find(|(i, _)| f.owners[*i].login == "corp-sso").unwrap();
    assert_eq!(term.backend().buffer()[(r.x + 6, r.y)].fg, Color::DarkGray);
    // Keys skip it...
    for _ in 0..6 {
        f.key(key(KeyCode::Down));
        assert_ne!(f.chosen().as_deref(), None);
        assert_ne!(f.owners[f.cursor].login, "corp-sso");
    }
    assert_eq!(f.chosen().as_deref(), Some("team-b"));
    // ...and a click only explains.
    f.click(click(r.x + 3, r.y), Rect::new(0, 0, 100, 16));
    assert_eq!(f.chosen().as_deref(), Some("team-b"));
    assert!(f.status.as_deref().unwrap().contains("SAML"));
}

#[test]
fn keyboard_and_mouse_select_an_entry() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut f = ready_picker(&s);
    f.key(key(KeyCode::Down));
    assert_eq!(f.chosen().as_deref(), Some("team-a"));
    f.key(key(KeyCode::Up));
    assert_eq!(f.chosen().as_deref(), Some("octo"));
    f.key(key(KeyCode::End));
    assert_eq!(f.chosen().as_deref(), Some("team-b"));
    f.key(key(KeyCode::Home));
    assert_eq!(f.chosen().as_deref(), Some("octo"));
    let l = f.layout(REGION);
    let (_, r) = l.entries.iter().find(|(i, _)| f.owners[*i].login == "team-a").unwrap();
    f.click(click(r.x + 2, r.y), REGION);
    assert_eq!(f.chosen().as_deref(), Some("team-a"));
    assert_eq!(f.key(key(KeyCode::Enter)), Nav::Next);
    assert_eq!(f.key(key(KeyCode::Esc)), Nav::Back);
}

#[test]
fn long_lists_scroll_to_keep_the_choice_visible() {
    let many: Vec<Owner> = std::iter::once(owner("octo", OwnerKind::User, true, None))
        .chain((0..30).map(|i| owner(&format!("org-{i}"), OwnerKind::Org, true, None)))
        .collect();
    let s = Setup::new(logged_in(), Ok(many));
    let mut f = ready_picker(&s);
    for _ in 0..25 {
        f.key(key(KeyCode::Down));
    }
    let l = f.layout(Rect::new(0, 0, 60, 8));
    assert!(l.entries.iter().any(|(i, _)| *i == f.cursor), "the chosen row is on screen");
    let t = draw(&f, 60, 8);
    assert!(t.contains("org-24"), "{t}");
}

#[test]
fn refresh_reloads_and_keeps_the_choice() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut f = ready_picker(&s);
    f.key(key(KeyCode::Down));
    *lock(&s.api.owners) = Ok(vec![owner("octo", OwnerKind::User, true, None), owner("team-a", OwnerKind::Org, true, None), owner("team-c", OwnerKind::Org, true, None)]);
    click_btn(&mut f, Btn::Refresh);
    assert!(matches!(f.phase, Phase::Loading(_)));
    settle(&mut f);
    assert_eq!(f.owners.len(), 3);
    assert_eq!(f.chosen().as_deref(), Some("team-a"));
    assert_eq!(*lock(&s.api.owner_calls), 2);
    f.key(key(KeyCode::Char('r')));
    settle(&mut f);
    assert_eq!(*lock(&s.api.owner_calls), 3);
}

// ---- create an organisation ----

#[test]
fn create_org_opens_github_waits_and_selects_the_new_org() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut f = ready_picker(&s);
    click_btn(&mut f, Btn::CreateOrg);
    assert_eq!(f.phase, Phase::OrgWait);
    assert_eq!(lock(&s.opened.0).last().unwrap(), "https://github.com/account/organizations/new?plan=free");
    let mut before = lock(&s.api.watch_before).clone();
    before.sort();
    assert_eq!(before, ["corp-sso", "team-a", "team-b"], "only organisations count as 'before'");
    let t = draw(&f, 80, 14);
    assert!(t.contains("Waiting for your new organisation..."), "{t}");
    for b in ["[ I'm done ]", "[ Cancel ]"] {
        assert!(t.contains(b), "{b}: {t}");
    }
    // The (fake) clock fires: the organisation appeared.
    let mut list = sample();
    list.push(owner("brand-new", OwnerKind::Org, true, None));
    *lock(&s.api.owners) = Ok(list);
    lock(&s.api.watch_tx).as_ref().unwrap().send(Some("brand-new".into())).unwrap();
    until(&mut f, "the picker", |f| f.phase == Phase::Picker);
    assert_eq!(f.chosen().as_deref(), Some("brand-new"));
}

#[test]
fn create_org_timeout_returns_to_the_list_with_a_hint() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut f = ready_picker(&s);
    f.key(key(KeyCode::Char('n')));
    lock(&s.api.watch_tx).as_ref().unwrap().send(None).unwrap();
    until(&mut f, "the picker", |f| f.phase == Phase::Picker);
    assert!(f.status.as_deref().unwrap().contains("Refresh"));
    assert_eq!(f.chosen().as_deref(), Some("octo"));
}

#[test]
fn im_done_refreshes_by_hand_and_cancel_stops_the_watch() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut f = ready_picker(&s);
    f.key(key(KeyCode::Char('n')));
    let stop = lock(&s.api.watch_stop).clone().unwrap();
    click_btn(&mut f, Btn::Done);
    assert!(stop.load(Ordering::SeqCst), "the watch was stopped");
    settle(&mut f);
    assert_eq!(f.phase, Phase::Picker);
    f.key(key(KeyCode::Char('n')));
    let stop2 = lock(&s.api.watch_stop).clone().unwrap();
    f.key(key(KeyCode::Esc));
    assert!(stop2.load(Ordering::SeqCst));
    settle(&mut f);
    assert_eq!(f.phase, Phase::Picker);
}

// ---- errors and fallback ----

#[test]
fn list_failure_is_plain_and_offers_retry_and_typing() {
    let s = Setup::new(logged_in(), Err("You seem to be offline.".into()));
    let mut f = s.flow();
    f.enter("");
    settle(&mut f);
    assert_eq!(f.phase, Phase::Manual);
    let t = draw_with_lines(&f);
    assert!(t.contains("You seem to be offline.") && t.contains("[ Try again ]") && t.contains("GitHub name"), "{t}");
    *lock(&s.api.owners) = Ok(sample());
    click_btn(&mut f, Btn::Retry);
    settle(&mut f);
    assert_eq!(f.phase, Phase::Picker);
}

#[test]
fn manual_entry_validates_and_pastes() {
    let s = Setup::new(logged_in(), Err("rate limited".into()));
    let mut f = s.flow();
    f.enter("");
    settle(&mut f);
    assert_eq!(f.chosen(), None);
    assert!(f.problem().contains("letters, numbers and dashes"));
    typed(&mut f, "bad name!");
    assert_eq!(f.chosen(), None, "spaces are not typed and '!' is invalid");
    f.key(KeyEvent { modifiers: KeyModifiers::CONTROL, ..key(KeyCode::Char('u')) });
    *lock(&s.clip.0) = " my-team\n".into();
    click_btn(&mut f, Btn::Paste);
    assert_eq!(f.chosen().as_deref(), Some("my-team"));
    f.paste("-x");
    assert_eq!(f.chosen().as_deref(), Some("my-team-x"));
    assert!(valid_owner("a-b") && !valid_owner("-a") && !valid_owner("a b") && !valid_owner(&"x".repeat(40)));
}

#[test]
fn type_a_name_instead_switches_to_the_box() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut f = ready_picker(&s);
    f.key(key(KeyCode::Char('t')));
    assert_eq!(f.phase, Phase::Manual);
    assert_eq!(f.manual, "octo", "starts from the highlighted account");
}

// ---- remembered choice ----

#[test]
fn the_choice_is_remembered_and_preselected_next_time() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut f = ready_picker(&s);
    f.key(key(KeyCode::Down));
    assert_eq!(f.commit().as_deref(), Some("team-a"));
    assert_eq!(load_choice(&s.store()).as_deref(), Some("team-a"));
    let f2 = ready_picker(&s);
    assert_eq!(f2.chosen().as_deref(), Some("team-a"));
}

#[test]
fn a_remembered_owner_that_is_gone_or_unusable_falls_back_to_the_first_usable() {
    let s = Setup::new(logged_in(), Ok(sample()));
    save_choice(&s.store(), "corp-sso");
    assert_eq!(ready_picker(&s).chosen().as_deref(), Some("octo"));
    save_choice(&s.store(), "vanished");
    assert_eq!(ready_picker(&s).chosen().as_deref(), Some("octo"));
    std::fs::write(s.store(), "not a name!!\n").unwrap();
    assert_eq!(ready_picker(&s).chosen().as_deref(), Some("octo"));
}

#[test]
fn the_website_suggestion_is_used_when_nothing_is_remembered() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut f = s.flow();
    f.enter("team-b");
    settle(&mut f);
    assert_eq!(f.chosen().as_deref(), Some("team-b"));
}

#[test]
fn going_back_stops_background_work() {
    let s = Setup::new(GhAuth::LoggedOut, Ok(sample()));
    let mut f = s.flow();
    f.enter("");
    until(&mut f, "the code", |f| matches!(f.phase, Phase::SignIn { .. }));
    f.leave();
    assert!(s.api.login_cancelled.load(Ordering::SeqCst));
    assert!(!f.busy());
    // Entering again starts over.
    f.enter("");
    until(&mut f, "a new code", |f| matches!(f.phase, Phase::SignIn { .. }));
}

#[test]
fn only_https_addresses_are_opened() {
    let c = url_command(OsKind::Other, "https://github.com/login/device").unwrap();
    assert_eq!((c.program.as_str(), c.args.len()), ("xdg-open", 1));
    assert_eq!(url_command(OsKind::Windows, "https://github.com/x").unwrap().program, "explorer.exe");
    assert_eq!(url_command(OsKind::Mac, "https://github.com/x").unwrap().program, "open");
    for bad in ["file:///etc/passwd", "http://x", "https://a b", "https://x\"&calc", "javascript:alert(1)"] {
        assert!(url_command(OsKind::Windows, bad).is_err(), "{bad}");
    }
}

#[test]
fn screens_render_at_small_sizes() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut f = ready_picker(&s);
    for (w, h) in [(20, 4), (40, 8), (60, 12), (120, 40), (3, 1)] {
        draw(&f, w, h);
    }
    f.key(key(KeyCode::Char('t')));
    for (w, h) in [(20, 4), (40, 8), (3, 1)] {
        draw(&f, w, h);
    }
}

// ---- wizard integration ----

struct AllFree;
impl PortProber for AllFree {
    fn tcp_free(&self, _: u16) -> bool {
        true
    }
    fn udp_free(&self, _: u16) -> bool {
        true
    }
}

fn wiz(s: &Setup, mode: Mode, with_gh: bool) -> Wizard {
    let mut w = Wizard::with_ports(PortEnv { prober: Box::new(AllFree), servers: vec![] });
    w.mode = mode;
    if with_gh {
        w = w.with_github(s.flow());
    }
    w
}

fn walk_to(w: &mut Wizard, target: Screen) {
    for _ in 0..40 {
        if w.screen == target {
            return;
        }
        w.go_next();
        assert!(w.error.is_none(), "{:?}: {:?}", w.screen, w.error);
    }
    assert_eq!(w.screen, target);
}

fn wizard_text(w: &Wizard, width: u16, height: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(width, height)).unwrap();
    t.draw(|f| crate::wizard_view::render(f, w)).unwrap();
    text_of(&t)
}

fn tick_until(w: &mut Wizard, what: &str, cond: impl Fn(&Wizard) -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !cond(w) {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        w.tick();
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn development_mode_shows_the_github_screen_and_the_owner_reaches_the_request_and_review() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut w = wiz(&s, Mode::Development, true);
    assert!(w.flow().contains(&Screen::GithubAccount));
    assert!(!w.flow().contains(&Screen::GithubOwner), "the text box is replaced, not doubled");
    walk_to(&mut w, Screen::GithubAccount);
    assert!(w.wants_tick(), "loading animates");
    tick_until(&mut w, "the list", |w| !w.wants_tick());
    let t = wizard_text(&w, 100, 30);
    assert!(t.contains("Where should your copies (forks) of every Kora script go?") && t.contains("octo (your account)"), "{t}");
    // Choose team-a by arrow key, then Next.
    w.handle_key(key(KeyCode::Down));
    w.handle_key(key(KeyCode::Enter));
    assert_ne!(w.screen, Screen::GithubAccount, "{:?}", w.error);
    assert_eq!(w.github_owner, "team-a");
    walk_to(&mut w, Screen::Review);
    let review = wizard_text(&w, 140, 60);
    assert!(review.contains("\"team-a\""), "Review names the owner: {review}");
    w.go_next();
    assert_eq!(w.state(), WizardState::Done);
    assert_eq!(w.build_request().github_owner.as_deref(), Some("team-a"));
    assert_eq!(load_choice(&s.store()).as_deref(), Some("team-a"));
}

#[test]
fn clicking_in_the_wizard_selects_and_the_buttons_work() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut w = wiz(&s, Mode::Development, true);
    walk_to(&mut w, Screen::GithubAccount);
    tick_until(&mut w, "the list", |w| !w.wants_tick());
    let area = Rect::new(0, 0, 100, 30);
    let l = crate::wizard_view::layout(area, &w.view());
    let g = w.gh.as_ref().unwrap().layout(crate::github_screens::region(&l));
    let (_, r) = g.entries.iter().find(|(i, _)| w.gh.as_ref().unwrap().owners[*i].login == "team-b").unwrap();
    w.handle_mouse(click(r.x + 2, r.y), area);
    assert_eq!(w.gh.as_ref().unwrap().chosen().as_deref(), Some("team-b"));
    // The footer Next button continues.
    w.handle_mouse(click(l.next.x + 1, l.next.y + 1), area);
    assert_eq!(w.github_owner, "team-b");
    assert_ne!(w.screen, Screen::GithubAccount);
}

#[test]
fn next_without_a_usable_choice_explains_and_stays() {
    let s = Setup::new(logged_in(), Err("offline".into()));
    let mut w = wiz(&s, Mode::Development, true);
    walk_to(&mut w, Screen::GithubAccount);
    tick_until(&mut w, "the error", |w| !w.wants_tick());
    w.handle_key(key(KeyCode::Enter));
    assert_eq!(w.screen, Screen::GithubAccount);
    assert!(w.error.as_deref().unwrap().contains("GitHub user or organisation name"));
    for c in "my-team".chars() {
        w.handle_key(key(KeyCode::Char(c)));
    }
    w.handle_paste("-2");
    w.handle_key(key(KeyCode::Enter));
    assert_ne!(w.screen, Screen::GithubAccount);
    assert_eq!(w.github_owner, "my-team-2");
}

#[test]
fn back_from_the_github_screen_cancels_the_sign_in() {
    let s = Setup::new(GhAuth::LoggedOut, Ok(sample()));
    let mut w = wiz(&s, Mode::Development, true);
    walk_to(&mut w, Screen::GithubAccount);
    tick_until(&mut w, "the code", |w| matches!(w.gh.as_ref().unwrap().phase, Phase::SignIn { .. }));
    let t = wizard_text(&w, 100, 30);
    assert!(t.contains("ABCD-1234") && t.contains("Waiting for you to approve in the browser..."), "{t}");
    w.handle_mouse(click(1, 28), Rect::new(0, 0, 100, 30)); // footer Back button area
    if w.screen == Screen::GithubAccount {
        w.go_back();
    }
    assert!(s.api.login_cancelled.load(Ordering::SeqCst));
}

#[test]
fn the_footer_shows_copy_confirmation_and_help_is_updated() {
    let s = Setup::new(GhAuth::LoggedOut, Ok(sample()));
    let mut w = wiz(&s, Mode::Development, true);
    walk_to(&mut w, Screen::GithubAccount);
    tick_until(&mut w, "the code", |w| matches!(w.gh.as_ref().unwrap().phase, Phase::SignIn { .. }));
    w.handle_key(key(KeyCode::Char('c')));
    assert!(w.status.as_deref().unwrap().contains("Code copied"));
    let h = w.view().help;
    for k in ["refresh", "N create a new organisation", "C copy the code", "grey"] {
        assert!(h.contains(k), "{k}: {h}");
    }
    w.handle_key(key(KeyCode::F(1)));
    assert!(w.help_open);
}

#[test]
fn production_mode_never_shows_the_github_screens() {
    let s = Setup::new(GhAuth::LoggedOut, Ok(sample()));
    let mut w = wiz(&s, Mode::Production, true);
    w.advanced = true;
    assert!(!w.flow().contains(&Screen::GithubAccount) && !w.flow().contains(&Screen::GithubOwner));
    walk_to(&mut w, Screen::Review);
    assert!(!w.gh.as_ref().unwrap().busy(), "nothing was started");
    assert_eq!(w.build_request().github_owner, None);
}

#[test]
fn a_setup_code_does_not_skip_the_github_screens_in_development_mode() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut w = wiz(&s, Mode::Development, true);
    w.apply_redeemed(
        kora_core::site::Redeemed {
            mode: Mode::Development,
            name: "Alpha One".into(),
            resources: kora_core::installer::ResourceSelection::All,
            artifact: kora_core::site::ArtifactChoice::Build(38621),
            artifact_url: None,
            game_build: None,
            ports: kora_core::installer::Ports { game: 31120, txadmin: 41120 },
            hostname: None,
            cfx_identifier: None,
            cfx_username: None,
            github_login: Some("team-b".into()),
        },
        true,
    );
    assert!(w.flow().contains(&Screen::GithubAccount));
    walk_to(&mut w, Screen::GithubAccount);
    tick_until(&mut w, "the list", |w| !w.wants_tick());
    assert_eq!(w.gh.as_ref().unwrap().chosen().as_deref(), Some("team-b"), "the website's suggestion is only a preselection");
}

#[test]
fn without_a_github_api_the_old_text_screen_is_used() {
    let s = Setup::new(logged_in(), Ok(sample()));
    let mut w = wiz(&s, Mode::Development, false);
    w.advanced = true;
    assert!(w.flow().contains(&Screen::GithubOwner) && !w.flow().contains(&Screen::GithubAccount));
}

#[test]
fn github_errors_are_put_in_plain_words() {
    use kora_core::git::GitError;
    let cmd = |stderr: &str| GitError::Command { cmd: "gh api user".into(), code: 1, stderr: stderr.into() };
    assert!(plain_error(&cmd("HTTP 429: secondary rate limit")).contains("limiting requests"));
    assert!(plain_error(&cmd("dial tcp: lookup api.github.com: no such host")).contains("internet connection"));
    assert!(plain_error(&cmd("HTTP 401: Bad credentials")).contains("not signed in"));
    assert_eq!(plain_error(&cmd("\nHTTP 500: oops\n")), "GitHub said: HTTP 500: oops");
    assert!(plain_error(&GitError::Spawn("gh".into(), "not found".into())).contains("couldn't run"));
}
