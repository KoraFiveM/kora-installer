//! Operation handlers, one module per area. See `crate::routes` for the Op → handler table.

pub mod system;
pub mod servers;
pub mod console;
pub mod updates;
pub mod resources;
pub mod settings;
pub mod github;
pub mod install;
pub mod maintenance;
pub mod remote;
