//! [`Host`] over the panel's [`Backend`]: stop and start the server the way the Start / Stop buttons do.

use super::Host;
use crate::api::{Backend, Status};
use crate::installer::registry::ServerEntry;
use crate::service::ControlAction;

pub struct BackendHost<'a, B: Backend + ?Sized> {
    backend: &'a mut B,
    entry: &'a ServerEntry,
}

impl<'a, B: Backend + ?Sized> BackendHost<'a, B> {
    pub fn new(backend: &'a mut B, entry: &'a ServerEntry) -> Self {
        BackendHost { backend, entry }
    }
}

impl<B: Backend + ?Sized> Host for BackendHost<'_, B> {
    fn running(&mut self) -> bool {
        self.backend.status(self.entry) == Status::Running
    }

    fn stop(&mut self) -> Result<(), String> {
        // A stop that was asked for is not a crash: the monitor must not raise an alert.
        crate::monitor::mark_stop_intent(&self.entry.dir);
        self.backend.control(self.entry, ControlAction::Stop).map_err(|m| crate::problem::plain_text(&m))
    }

    fn start(&mut self) -> Result<(), String> {
        self.backend.control(self.entry, ControlAction::Start).map_err(|m| crate::problem::plain_text(&m))
    }
}
