//! One module per command area: its clap definitions, its handlers and its tests.
//! See CONTRIBUTING.md ("Adding a command").
pub(crate) mod app;
pub(crate) mod github;
pub(crate) mod helper;
pub(crate) mod install;
pub(crate) mod resources;
pub(crate) mod schedule;
pub(crate) mod server;
pub(crate) mod uninstall;
pub(crate) mod update;
pub(crate) mod webhook;
