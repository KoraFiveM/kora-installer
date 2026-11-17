//! Kora core library.
//!
//! Implemented in M0: [`steps`] (step engine trait + persisted state).
//! Everything else is a documented stub filled in during M2.

pub mod addresources;
pub mod api;
pub mod app_install;
pub mod artifacts;
pub mod catalog;
pub mod cfx;
pub mod console;
pub mod db;
pub mod elevate;
pub mod firewall;
pub mod feedback;
pub mod fsutil;
pub mod helper;
pub mod git;
pub mod hub;
pub mod service;
pub mod space;
pub mod steps;
pub mod sysexe;
pub mod txadmin;
pub mod update;
pub mod userhome;
pub mod installer;
pub mod launch;
pub mod monitor;
pub mod notify;
pub mod paidassets;
pub mod problem;
pub mod self_update;
