//! One module per command area: its clap definitions, its handlers and its tests.
//! See CONTRIBUTING.md ("Adding a command").
pub(crate) mod admin_worker;
pub(crate) mod app;
pub(crate) mod bare;
pub(crate) mod branch;
pub(crate) mod desktop;
pub(crate) mod github;
pub(crate) mod helper;
pub(crate) mod install;
pub(crate) mod keys_cmd;
pub(crate) mod panel;
pub(crate) mod panel_background;
pub(crate) mod panel_remote;
pub(crate) mod panel_running;
pub(crate) mod panel_tailscale;
pub(crate) mod reach;
pub(crate) mod resource;
pub(crate) mod resources;
pub(crate) mod schedule;
pub(crate) mod server;
pub(crate) mod store;
pub(crate) mod toolchain;
pub(crate) mod uninstall;
pub(crate) mod update;
pub(crate) mod webhook;
