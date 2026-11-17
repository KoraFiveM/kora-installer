//! Wire types: the JSON the HTTP API sends and receives, one struct or enum per shape.
//!
//! Each derives `Serialize` + `Deserialize` (so handlers and the contract test use the very same
//! type) and, with the `ts` feature, `ts_rs::TS` (so the web panel's TypeScript is generated from it).
//! Put new shapes in a file of their own per area. Rules: snake_case on the wire, never a secret in
//! a field, `#[serde(default, skip_serializing_if = "Option::is_none")]` plus `#[ts(optional)]` for a
//! field that is left out (TypeScript gets `field?:`), a plain `Option` for a field that is sent as `null`.

mod audit;
mod auth;
mod common;
mod install;
mod keys;
mod more;
mod reach;
mod servers;
mod session;
mod system;
mod updates;

pub use audit::*;
pub use auth::*;
pub use common::*;
pub use install::*;
pub use keys::*;
pub use more::*;
pub use reach::*;
pub use servers::*;
pub use session::*;
pub use system::*;
pub use updates::*;
