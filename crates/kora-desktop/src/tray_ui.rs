//! The tray icon: turns the plain menu model of `kora_shell::tray` into native menu items and keeps the icon,
//! tooltip and menu up to date. No decisions here.

use kora_shell::icon;
use kora_shell::tray::{Entry, IconKind};
use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

const SIZE: u32 = 32;

fn icon_for(kind: IconKind) -> Option<Icon> {
    Icon::from_rgba(icon::status_icon(kind, SIZE), SIZE, SIZE).ok()
}

/// The window/taskbar icon (the Kora lime badge).
pub fn window_icon() -> Option<tao::window::Icon> {
    let size = 64;
    tao::window::Icon::from_rgba(icon::status_icon(IconKind::Green, size), size, size).ok()
}

fn items(entries: &[Entry]) -> Vec<Box<dyn IsMenuItem>> {
    let mut out: Vec<Box<dyn IsMenuItem>> = Vec::new();
    for e in entries {
        match e {
            Entry::Item { id, label, enabled } => out.push(Box::new(MenuItem::with_id(id.as_str(), label, *enabled, None))),
            Entry::Check { id, label, checked } => out.push(Box::new(CheckMenuItem::with_id(id.as_str(), label, true, *checked, None))),
            Entry::Info(text) => out.push(Box::new(MenuItem::new(text, false, None))),
            Entry::Separator => out.push(Box::new(PredefinedMenuItem::separator())),
            Entry::Submenu { label, items: children } => {
                let kids = items(children);
                let refs: Vec<&dyn IsMenuItem> = kids.iter().map(|b| b.as_ref()).collect();
                if let Ok(sub) = Submenu::with_items(label, true, &refs) {
                    out.push(Box::new(sub));
                }
            }
        }
    }
    out
}

pub fn build_menu(entries: &[Entry]) -> Menu {
    let menu = Menu::new();
    for item in items(entries) {
        let _ = menu.append(item.as_ref());
    }
    menu
}

/// The tray icon with what it currently shows (so it is only touched when something changed).
pub struct Tray {
    icon: TrayIcon,
    kind: IconKind,
    tooltip: String,
    entries: Vec<Entry>,
}

impl Tray {
    /// `Err` when there is no notification area / tray host (a Linux desktop without one): the app then runs
    /// with a normal window and no tray.
    pub fn new(kind: IconKind, tooltip: &str, entries: Vec<Entry>) -> Result<Tray, String> {
        let mut b = TrayIconBuilder::new().with_menu(Box::new(build_menu(&entries))).with_tooltip(tooltip).with_menu_on_left_click(false);
        if let Some(i) = icon_for(kind) {
            b = b.with_icon(i);
        }
        let icon = b.build().map_err(|e| e.to_string())?;
        Ok(Tray { icon, kind, tooltip: tooltip.to_string(), entries })
    }

    pub fn update(&mut self, kind: IconKind, tooltip: &str, entries: Vec<Entry>) {
        if kind != self.kind {
            if let Some(i) = icon_for(kind) {
                let _ = self.icon.set_icon(Some(i));
            }
            self.kind = kind;
        }
        if tooltip != self.tooltip {
            let _ = self.icon.set_tooltip(Some(tooltip));
            self.tooltip = tooltip.to_string();
        }
        if entries != self.entries {
            self.icon.set_menu(Some(Box::new(build_menu(&entries))));
            self.entries = entries;
        }
    }
}
