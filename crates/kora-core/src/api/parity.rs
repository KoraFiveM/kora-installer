//! How a front-end feature relates to the shared operation list ([`super::Op`]). The TUI and CLI
//! parity tests use this so a feature cannot exist in one front end without the panel knowing.

use super::Op;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    /// Served by this operation.
    Route(Op),
    /// Purely about the screen it is on (navigation, scrolling, quitting...): nothing to serve.
    UiOnly(&'static str),
}

impl Parity {
    pub fn op(self) -> Option<Op> {
        match self {
            Parity::Route(o) => Some(o),
            Parity::UiOnly(_) => None,
        }
    }
}
