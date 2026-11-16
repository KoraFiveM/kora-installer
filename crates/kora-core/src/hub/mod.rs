//! The hub: any panel can list other Kora panels ("nodes") and show or control their servers.
//! There is no central service; a node is just another panel reached over its `/api/v1` with a bearer token.
//!
//! * [`config`]: the nodes file (`hub.json`) and URL checks.
//! * [`remote`]: [`RemoteBackend`], a client for one node, with version-skew handling.
//! * [`transport`]: the HTTP seam ([`Transport`]) and its `ureq` implementation.

pub mod config;
pub mod remote;
pub mod transport;

pub use config::{check_url, NodeConfig, NodeStore, UrlCheck};
pub use remote::{Meta, RemoteBackend, RemoteError};
pub use transport::{HttpRequest, HttpResponse, Transport, UreqTransport};
