//! State of the "Add resources" and "Report an issue" / "Open a pull request" screens.
//! No terminal access and no system calls in here: `app.rs` runs the jobs, `view.rs` draws.

use crossterm::event::{KeyCode, KeyEvent};
use kora_core::addresources::{self as add, AddOutcome, AddPlan};
use kora_core::catalog::{Catalog, PickError, Picker, Section};
use kora_core::feedback::PrPlan;
use kora_core::installer::registry::ServerEntry;

use super::app::{Action, Job, Level};
use super::input::TextInput;

/// What a screen asks `App` to do after a key or click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    None,
    Say(Level, String),
    Job(Job),
    /// Leave the screen.
    Close,
}

// ------------------------------------------------------------------ add resources

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddResStage {
    Menu,
    /// Typing a website code.
    Code,
    /// The three catalog sections.
    Sections,
    /// A list of resources (or creators) inside a section.
    Items,
    /// What will be installed, waiting for the go-ahead.
    Review,
    Result,
}

/// What the `Items` stage lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListView {
    Section(Section),
    /// The creators (each opens its own resources).
    CreatorIndex,
    Creator(String),
}

/// One line of a list: label, optional checkbox state, a note, and what Enter does with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowView {
    pub label: String,
    pub checked: Option<bool>,
    pub note: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AddResState {
    pub entry: ServerEntry,
    pub stage: AddResStage,
    pub cat: Option<Catalog>,
    pub catalog_note: Option<String>,
    pub installed: Vec<String>,
    /// Explicit picks (catalog ids). Dependencies are derived, never stored.
    pub picked: Vec<String>,
    pub code: TextInput,
    pub sel: usize,
    pub view: ListView,
    pub plan: Option<AddPlan>,
    pub outcome: Option<AddOutcome>,
    pub from_code: bool,
}

impl AddResState {
    pub fn new(entry: ServerEntry) -> Self {
        Self {
            entry,
            stage: AddResStage::Menu,
            cat: None,
            catalog_note: None,
            installed: vec![],
            picked: vec![],
            code: TextInput::new(40),
            sel: 0,
            view: ListView::Section(Section::Compat),
            plan: None,
            outcome: None,
            from_code: false,
        }
    }

    fn picker(&self) -> Option<Picker<'_>> {
        let cat = self.cat.as_ref()?;
        Picker::with_ids(cat, self.installed.clone(), &self.picked).ok()
    }

    /// Lines of the section list (Sections stage): title, count and a final "review" row.
    pub fn section_rows(&self) -> Vec<RowView> {
        let Some(cat) = &self.cat else { return vec![] };
        let mut v: Vec<RowView> = Section::ALL
            .iter()
            .map(|s| {
                let (n, what) = match s {
                    Section::Creators => (cat.creators_with_resources().len(), "creators"),
                    _ => (cat.section(*s).len(), "available"),
                };
                RowView {
                    label: s.title().to_string(),
                    checked: None,
                    note: format!("{n} {what}"),
                    id: None,
                }
            })
            .collect();
        v.push(RowView {
            label: format!("Review and install ({} chosen)", self.picked.len()),
            checked: None,
            note: String::new(),
            id: None,
        });
        v
    }

    /// Lines of the current list (Items stage).
    pub fn item_rows(&self) -> Vec<RowView> {
        let (Some(cat), Some(p)) = (&self.cat, self.picker()) else {
            return vec![];
        };
        let res_rows = |list: Vec<&kora_core::catalog::Resource>| -> Vec<RowView> {
            list.into_iter()
                .map(|r| {
                    let note = if p.is_installed(&r.id) {
                        "installed".to_string()
                    } else if !p.is_picked(&r.id) && p.is_checked(&r.id) {
                        format!("needed by {}", p.needed_by(&r.id).join(", "))
                    } else if p.is_picked(&r.id) && !p.needed_by(&r.id).is_empty() {
                        format!("also needed by {}", p.needed_by(&r.id).join(", "))
                    } else {
                        String::new()
                    };
                    RowView {
                        label: r.title.clone().unwrap_or_else(|| r.id.clone()),
                        checked: Some(p.is_checked(&r.id)),
                        note,
                        id: Some(r.id.clone()),
                    }
                })
                .collect()
        };
        match &self.view {
            ListView::Section(s) => res_rows(cat.section(*s)),
            ListView::Creator(c) => res_rows(cat.creator_resources(c)),
            ListView::CreatorIndex => cat
                .creators_with_resources()
                .into_iter()
                .map(|c| {
                    let list = cat.creator_resources(&c.id);
                    let on = list.iter().filter(|r| p.is_checked(&r.id)).count();
                    RowView {
                        label: c.name.clone(),
                        checked: None,
                        note: format!("{} resources, {on} chosen", list.len()),
                        id: Some(c.id.clone()),
                    }
                })
                .collect(),
        }
    }

    pub fn row_count(&self) -> usize {
        match self.stage {
            AddResStage::Sections => self.section_rows().len(),
            AddResStage::Items => self.item_rows().len(),
            _ => 0,
        }
    }

    pub fn list_title(&self) -> String {
        match &self.view {
            ListView::Section(s) => s.title().to_string(),
            ListView::CreatorIndex => "Creators".into(),
            ListView::Creator(id) => self
                .cat
                .as_ref()
                .and_then(|c| c.creators.iter().find(|x| &x.id == id))
                .map(|c| c.name.clone())
                .unwrap_or_else(|| id.clone()),
        }
    }

    /// The description shown above a creator's list (who they are and where to find them).
    pub fn creator_blurb(&self) -> Option<String> {
        let ListView::Creator(id) = &self.view else {
            return None;
        };
        let c = self.cat.as_ref()?.creators.iter().find(|c| &c.id == id)?;
        Some(
            [c.description.clone(), c.url.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("  "),
        )
    }

    pub fn move_sel(&mut self, delta: isize) {
        let n = self.row_count();
        if n > 0 {
            self.sel = (self.sel as isize + delta).clamp(0, n as isize - 1) as usize;
        }
    }

    fn pick_msg(e: &PickError) -> Effect {
        match e {
            PickError::Needed { id, by } => Effect::Say(
                Level::Info,
                format!(
                    "'{id}' is needed by {}, so it stays selected. Deselect that first.",
                    by.join(", ")
                ),
            ),
            PickError::Installed(id) => {
                Effect::Say(Level::Info, format!("'{id}' is already installed."))
            }
            other => Effect::Say(Level::Bad, other.to_string()),
        }
    }

    /// Space / click on a resource row.
    pub fn toggle(&mut self) -> Effect {
        if self.stage != AddResStage::Items {
            return Effect::None;
        }
        let Some(id) = self
            .item_rows()
            .get(self.sel)
            .and_then(|r| r.checked.and(r.id.clone()))
        else {
            return Effect::None;
        };
        let Some(cat) = self.cat.as_ref() else {
            return Effect::None;
        };
        let Ok(mut p) = Picker::with_ids(cat, self.installed.clone(), &self.picked) else {
            return Effect::None;
        };
        match p.toggle(&id) {
            Ok(_) => {
                self.picked = p.picked_ids();
                Effect::None
            }
            Err(e) => Self::pick_msg(&e),
        }
    }

    /// Select every resource of the list, or clear them when all are already chosen.
    pub fn toggle_all(&mut self) -> Effect {
        if self.stage != AddResStage::Items {
            return Effect::None;
        }
        let ids: Vec<String> = self
            .item_rows()
            .into_iter()
            .filter(|r| r.checked.is_some())
            .filter_map(|r| r.id)
            .collect();
        let Some(cat) = self.cat.as_ref() else {
            return Effect::None;
        };
        let Ok(mut p) = Picker::with_ids(cat, self.installed.clone(), &self.picked) else {
            return Effect::None;
        };
        let refs = || ids.iter().map(String::as_str);
        let mut fx = Effect::None;
        if p.all_checked(refs()) {
            let blocked = p.deselect_all(refs());
            if let Some(e) = blocked.first() {
                fx = Effect::Say(
                    Level::Info,
                    format!(
                        "{} stayed selected because something else needs {}. {}",
                        blocked.len(),
                        if blocked.len() == 1 { "it" } else { "them" },
                        Self::note_of(e)
                    ),
                );
            }
        } else {
            p.select_all(refs());
        }
        self.picked = p.picked_ids();
        fx
    }

    fn note_of(e: &PickError) -> String {
        match e {
            PickError::Needed { id, by } => format!("({id} is needed by {})", by.join(", ")),
            other => other.to_string(),
        }
    }

    /// Enter on a row.
    pub fn open(&mut self) -> Effect {
        match self.stage {
            AddResStage::Sections => match self.sel {
                0 => self.enter(ListView::Section(Section::Compat)),
                1 => self.enter(ListView::Section(Section::Bridges)),
                2 => self.enter(ListView::CreatorIndex),
                _ => return self.review(),
            },
            AddResStage::Items => match (&self.view, self.item_rows().get(self.sel)) {
                (ListView::CreatorIndex, Some(r)) => {
                    if let Some(id) = r.id.clone() {
                        self.enter(ListView::Creator(id));
                    }
                }
                (_, Some(r)) if r.checked.is_some() => return self.toggle(),
                _ => {}
            },
            _ => {}
        }
        Effect::None
    }

    fn enter(&mut self, v: ListView) {
        self.view = v;
        self.stage = AddResStage::Items;
        self.sel = 0;
    }

    /// Build the plan for what is picked and show it.
    pub fn review(&mut self) -> Effect {
        let Some(p) = self.picker() else {
            return Effect::None;
        };
        if p.is_empty() {
            return Effect::Say(
                Level::Info,
                "Nothing is chosen yet. Pick at least one resource (Space).".into(),
            );
        }
        let server_file_stub = self.stub_server();
        match add::plan_from_picker(&p, &server_file_stub, &self.picked) {
            Ok(plan) => {
                self.plan = Some(plan);
                self.stage = AddResStage::Review;
                Effect::None
            }
            Err(e) => Effect::Say(Level::Bad, e.to_string()),
        }
    }

    /// Only the mode matters to planning; the real server file is read again when installing.
    fn stub_server(&self) -> kora_core::installer::registry::ServerFile {
        kora_core::installer::registry::ServerFile {
            name: self.entry.name.clone(),
            mode: self.entry.mode,
            dir: self.entry.dir.clone(),
            ports: self.entry.ports,
            game_build: 0,
            artifact_build: 0,
            database: None,
            github_owner: None,
            created_unix: 0,
            kora_version: String::new(),
            resources: vec![],
        }
    }

    /// Esc. Returns `false` when the screen should close.
    pub fn back(&mut self) -> bool {
        match (self.stage, &self.view) {
            (AddResStage::Menu | AddResStage::Result, _) => false,
            (AddResStage::Code | AddResStage::Sections, _) => {
                self.stage = AddResStage::Menu;
                true
            }
            (AddResStage::Items, ListView::Creator(_)) => {
                self.view = ListView::CreatorIndex;
                self.sel = 0;
                true
            }
            (AddResStage::Items, _) => {
                self.stage = AddResStage::Sections;
                self.sel = 0;
                true
            }
            (AddResStage::Review, _) => {
                self.stage = if self.from_code {
                    AddResStage::Menu
                } else {
                    AddResStage::Sections
                };
                true
            }
        }
    }

    /// Use the ids a website code returned. Unknown ids are refused as a whole.
    pub fn use_code_ids(&mut self, ids: &[String]) -> Effect {
        let Some(cat) = self.cat.as_ref() else {
            return Effect::None;
        };
        match Picker::with_ids(cat, self.installed.clone(), ids) {
            Ok(p) => {
                self.picked = p.picked_ids();
                self.from_code = true;
                if self.picked.is_empty() {
                    return Effect::Say(Level::Info, "Everything from that code is already installed.".into());
                }
                self.review()
            }
            Err(e) => Effect::Say(Level::Bad, format!("That code lists something this catalog does not offer ({e}). Update Kora, or pick by hand with Browse.")),
        }
    }

    pub fn key(&mut self, k: KeyEvent) -> Option<Action> {
        use AddResStage as S;
        match (self.stage, k.code) {
            (_, KeyCode::Esc) => Some(Action::Back),
            (S::Menu, KeyCode::Char('c')) => Some(Action::UseCode),
            (S::Menu, KeyCode::Char('b')) => Some(Action::Browse),
            (S::Code, KeyCode::Enter) => Some(Action::Confirm),
            (S::Code, _) => {
                self.code.key(k);
                None
            }
            (S::Sections | S::Items, KeyCode::Up | KeyCode::Char('k')) => Some(Action::Up),
            (S::Sections | S::Items, KeyCode::Down | KeyCode::Char('j')) => Some(Action::Down),
            (S::Sections | S::Items, KeyCode::Enter | KeyCode::Right) => Some(Action::Confirm),
            (S::Items, KeyCode::Char(' ')) => Some(Action::Toggle),
            (S::Items, KeyCode::Char('a')) => Some(Action::ToggleAll),
            (S::Sections | S::Items, KeyCode::Char('i')) => Some(Action::Install),
            (S::Review, KeyCode::Enter | KeyCode::Char('y')) => Some(Action::Confirm),
            (S::Review, KeyCode::Char('n')) => Some(Action::Back),
            _ => None,
        }
    }

    /// Run one action; `Effect` says what App must do next.
    pub fn act(&mut self, a: Action) -> Effect {
        use AddResStage as S;
        match (self.stage, a) {
            (S::Menu, Action::UseCode) => {
                self.stage = S::Code;
                Effect::None
            }
            (S::Menu, Action::Browse) => {
                self.stage = S::Sections;
                self.sel = 0;
                Effect::None
            }
            (S::Code, Action::Confirm) => {
                if self.code.text().trim().is_empty() {
                    Effect::Say(
                        Level::Info,
                        "Type or paste the code from the website (KORA-XXXX-XXXX).".into(),
                    )
                } else {
                    Effect::Job(Job::AddRedeem)
                }
            }
            (S::Sections | S::Items, Action::Up) => {
                self.move_sel(-1);
                Effect::None
            }
            (S::Sections | S::Items, Action::Down) => {
                self.move_sel(1);
                Effect::None
            }
            (S::Sections | S::Items, Action::Confirm) => self.open(),
            (S::Sections | S::Items, Action::Install) => self.review(),
            (S::Items, Action::Toggle) => self.toggle(),
            (S::Items, Action::ToggleAll) => self.toggle_all(),
            (S::Review, Action::Confirm) => {
                if self.plan.as_ref().is_some_and(|p| !p.is_empty()) {
                    Effect::Job(Job::AddInstall)
                } else {
                    Effect::Say(
                        Level::Info,
                        "Nothing to install: everything is already there.".into(),
                    )
                }
            }
            (_, Action::Back) => {
                if self.back() {
                    Effect::None
                } else {
                    Effect::Close
                }
            }
            _ => Effect::None,
        }
    }

    /// A click on row `i` (checkbox rows toggle, others open).
    pub fn click_row(&mut self, i: usize) -> Effect {
        self.sel = i.min(self.row_count().saturating_sub(1));
        self.open()
    }
}

// ------------------------------------------------------------------ report issue / open PR

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbKind {
    Issue,
    Pr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbStage {
    Pick,
    Compose,
    Done,
}

#[derive(Debug, Clone)]
pub struct FeedbackState {
    pub entry: ServerEntry,
    pub kind: FbKind,
    pub stage: FbStage,
    pub choices: Vec<String>,
    pub sel: usize,
    pub resource: String,
    pub title: TextInput,
    pub body: TextInput,
    /// 0 = title, 1 = text.
    pub focus: usize,
    pub plan: Option<PrPlan>,
    pub url: Option<String>,
}

impl FeedbackState {
    pub fn new(entry: ServerEntry, kind: FbKind, choices: Vec<String>) -> Self {
        Self {
            entry,
            kind,
            stage: FbStage::Pick,
            choices,
            sel: 0,
            resource: String::new(),
            title: TextInput::new(200),
            body: TextInput::new(2000),
            focus: 0,
            plan: None,
            url: None,
        }
    }

    pub fn heading(&self) -> &'static str {
        match self.kind {
            FbKind::Issue => "Report an issue",
            FbKind::Pr => "Open a pull request",
        }
    }

    pub fn key(&mut self, k: KeyEvent) -> Option<Action> {
        match (self.stage, k.code) {
            (_, KeyCode::Esc) => Some(Action::Back),
            (FbStage::Pick, KeyCode::Up | KeyCode::Char('k')) => Some(Action::Up),
            (FbStage::Pick, KeyCode::Down | KeyCode::Char('j')) => Some(Action::Down),
            (FbStage::Pick, KeyCode::Enter) => Some(Action::Confirm),
            (FbStage::Compose, KeyCode::Tab | KeyCode::BackTab) => Some(Action::NextField),
            (FbStage::Compose, KeyCode::Enter) => Some(Action::Confirm),
            (FbStage::Compose, _) => {
                if self.focus == 0 {
                    self.title.key(k)
                } else {
                    self.body.key(k)
                };
                None
            }
            (FbStage::Done, KeyCode::Char('c')) => Some(Action::Copy),
            _ => None,
        }
    }

    pub fn act(&mut self, a: Action) -> Effect {
        match (self.stage, a) {
            (FbStage::Pick, Action::Up) => {
                self.sel = self.sel.saturating_sub(1);
                Effect::None
            }
            (FbStage::Pick, Action::Down) => {
                self.sel = (self.sel + 1).min(self.choices.len().saturating_sub(1));
                Effect::None
            }
            (FbStage::Pick, Action::Confirm) => match self.choices.get(self.sel) {
                None => Effect::Say(Level::Info, "This server has no recorded resources.".into()),
                Some(r) => {
                    self.resource = r.clone();
                    match self.kind {
                        FbKind::Issue => {
                            self.stage = FbStage::Compose;
                            Effect::None
                        }
                        FbKind::Pr => Effect::Job(Job::PrPlan),
                    }
                }
            },
            (FbStage::Compose, Action::NextField) => {
                self.focus = 1 - self.focus;
                Effect::None
            }
            (FbStage::Compose, Action::Confirm) => {
                if self.title.text().trim().is_empty() {
                    self.focus = 0;
                    return Effect::Say(Level::Info, "Give it a short title first.".into());
                }
                Effect::Job(match self.kind {
                    FbKind::Issue => Job::IssueSend,
                    FbKind::Pr => Job::PrOpen,
                })
            }
            (FbStage::Compose, Action::Back) => {
                self.stage = FbStage::Pick;
                Effect::None
            }
            (_, Action::Back) => Effect::Close,
            _ => Effect::None,
        }
    }
}
