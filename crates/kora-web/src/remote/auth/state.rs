//! In-memory state of the sign-in ceremonies in flight, and the rules that decide who may use which credential.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use kora_core::api::Op;

use super::passkey::Ceremonies;
use crate::remote::Provider;

/// A one-time-code set-up waits this long for its first code.
pub const TOTP_PENDING_TTL: Duration = Duration::from_secs(10 * 60);

/// Everything the sign-in handlers keep between two requests. Nothing here is saved: a restart makes a person
/// start the step again.
#[derive(Default)]
pub struct AuthState {
    pub ceremonies: Ceremonies,
    /// Secrets handed out for a code set-up, until the first code proves the app has them. Keyed by who they were
    /// made for (an enrolment link's hash, or `account:<user>`).
    totp: Mutex<HashMap<String, (Instant, [u8; 20])>>,
}

impl AuthState {
    pub fn with_ceremonies(c: Ceremonies) -> Self {
        Self { ceremonies: c, totp: Mutex::default() }
    }
    pub fn put_totp(&self, key: &str, secret: [u8; 20]) {
        let mut g = self.totp.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.retain(|_, (t, _)| t.elapsed() < TOTP_PENDING_TTL);
        if g.len() >= 256 {
            g.clear();
        }
        g.insert(key.to_string(), (Instant::now(), secret));
    }
    /// The pending secret (it stays until [`drop_totp`](Self::drop_totp), so a mistyped code can be tried again).
    pub fn get_totp(&self, key: &str) -> Option<[u8; 20]> {
        let g = self.totp.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.get(key).filter(|(t, _)| t.elapsed() < TOTP_PENDING_TTL).map(|(_, s)| *s)
    }
    pub fn drop_totp(&self, key: &str) {
        self.totp.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(key);
    }
}

/// How an interactive token sign-in is judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSignIn {
    /// Fine. `upgrade`: the person has no passkey or one-time codes yet, so the screen suggests them.
    Allowed {
        upgrade: bool,
    },
    Refused(&'static str),
}

pub const TOKEN_REFUSED_STRONG: &str = "An administrator requires a passkey or one-time codes for signing in, so the token link no longer works for this. Ask for a new enrolment link.";
pub const TOKEN_REFUSED_HAS_STRONG: &str = "You have set up a passkey or one-time codes, so the old token link is not accepted for signing in any more. Sign in with your passkey or codes.";

/// May a person sign in with the old token link? Never when an administrator requires strong sign-in, and never
/// once the user has a passkey or one-time codes (so a leaked old link cannot bypass them).
pub fn token_sign_in(require_strong: bool, user_has_strong: bool) -> TokenSignIn {
    if require_strong {
        TokenSignIn::Refused(TOKEN_REFUSED_STRONG)
    } else if user_has_strong {
        TokenSignIn::Refused(TOKEN_REFUSED_HAS_STRONG)
    } else {
        TokenSignIn::Allowed { upgrade: true }
    }
}

/// The actions that, for a remote session that signed in with a passkey or one-time codes, need a passkey or code check
/// made in the last [`crate::security::FRESH_FOR`]: deleting, uninstalling, anything that opens full control of the game
/// server, and everything that changes how people sign in. A stolen cookie (even with its secret) is then not enough.
///
/// This list is about the actions a REMOTE session may reach at all (user management, TLS and remote settings are
/// local-only already, see `roles::local_only`).
pub fn needs_fresh(op: Op) -> bool {
    // Every destructive operation (so a new one is covered the day it is added), plus the ones named here.
    crate::remote::roles::destructive(op)
        || matches!(
            op,
            Op::ServerDelete | Op::ServerUnregister | Op::UninstallRun | Op::AppUninstall | Op::SelfUpdateApply | Op::TxAdminTicket | Op::ConsoleSend | Op::ScheduleSet | Op::ScheduleRemove
        )
}

/// Does this provider's session get the fresh-check rule? (Tailscale identity is the machine's, not a credential the
/// page could ask for again; the launch token session is the computer itself; API tokens are scripts.)
pub fn reauth_applies(p: Provider) -> bool {
    matches!(p, Provider::Passkey | Provider::Code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::roles;

    #[test]
    fn every_destructive_action_asks_for_a_fresh_check() {
        for op in Op::ALL {
            if roles::destructive(*op) {
                assert!(needs_fresh(*op), "{op:?} is destructive but does not need a fresh check");
            }
        }
        assert!(!needs_fresh(Op::ServersList) && !needs_fresh(Op::ServerControl));
    }

    #[test]
    fn the_token_cannot_downgrade_a_strong_account() {
        assert_eq!(token_sign_in(false, false), TokenSignIn::Allowed { upgrade: true });
        assert!(matches!(token_sign_in(true, false), TokenSignIn::Refused(m) if m.contains("requires")));
        assert!(matches!(token_sign_in(false, true), TokenSignIn::Refused(m) if m.contains("passkey or one-time codes")));
        assert!(matches!(token_sign_in(true, true), TokenSignIn::Refused(_)));
    }

    #[test]
    fn pending_secrets_live_until_dropped() {
        let s = AuthState::default();
        s.put_totp("k", [7; 20]);
        assert_eq!(s.get_totp("k"), Some([7; 20]));
        assert_eq!(s.get_totp("k"), Some([7; 20]), "a mistyped code can be tried again");
        s.drop_totp("k");
        assert_eq!(s.get_totp("k"), None);
    }

    #[test]
    fn only_strong_sessions_are_asked() {
        assert!(reauth_applies(Provider::Passkey) && reauth_applies(Provider::Code));
        assert!(!reauth_applies(Provider::Tailscale) && !reauth_applies(Provider::Local) && !reauth_applies(Provider::Token) && !reauth_applies(Provider::ApiToken));
    }
}
