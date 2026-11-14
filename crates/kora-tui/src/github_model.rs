//! GitHub account types used by the sign-in and owner screens.
//!
//! TEMPORARY local copy of `kora_core::git::owners` (same names and shapes). Once that module is
//! on main this file becomes a plain `pub use`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerKind {
    User,
    Org,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub login: String,
    pub kind: OwnerKind,
    pub can_fork: bool,
    pub reason: Option<String>,
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhAuth {
    Missing,
    LoggedOut,
    LoggedIn { login: String },
}
