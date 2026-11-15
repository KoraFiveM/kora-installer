//! Kora core library.
//!
//! Implemented in M0: [`steps`] (step engine trait + persisted state).
//! Everything else is a documented stub filled in during M2.

pub mod app_install;
pub mod artifacts;
pub mod catalog;
pub mod db;
pub mod elevate;
pub mod firewall;
pub mod fsutil;
pub mod git;
pub mod service;
pub mod site;
pub mod space;
pub mod steps;
pub mod sysexe;
pub mod txadmin;
pub mod update;
pub mod installer;
pub mod launch;
pub mod notify;
