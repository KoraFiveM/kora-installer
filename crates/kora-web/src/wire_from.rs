//! Conversions from the panel's own types (`remote::Role`, `Provider`, `Identity`) to the wire types
//! of `kora_core::api::wire`, which the TypeScript is generated from.
use kora_core::api::wire;

use crate::remote::{Identity, Provider, Role};

impl From<Role> for wire::Role {
    fn from(r: Role) -> Self {
        match r {
            Role::Viewer => wire::Role::Viewer,
            Role::Operator => wire::Role::Operator,
            Role::Admin => wire::Role::Admin,
        }
    }
}

impl From<Provider> for wire::Provider {
    fn from(p: Provider) -> Self {
        match p {
            Provider::Local => wire::Provider::Local,
            Provider::Tailscale => wire::Provider::Tailscale,
            Provider::Token => wire::Provider::Token,
        }
    }
}

impl From<&Identity> for wire::Identity {
    fn from(i: &Identity) -> Self {
        wire::Identity { name: i.name.clone(), provider: i.provider.into(), role: i.role.into() }
    }
}

impl From<wire::Role> for Role {
    fn from(r: wire::Role) -> Self {
        match r {
            wire::Role::Viewer => Role::Viewer,
            wire::Role::Operator => Role::Operator,
            wire::Role::Admin => Role::Admin,
        }
    }
}
