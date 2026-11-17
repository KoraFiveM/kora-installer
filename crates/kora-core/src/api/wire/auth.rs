//! Remote sign-in (passkeys, one-time codes, enrolment links, API tokens) and HTTPS for the panel.
//! Never a secret at rest in an answer: a secret appears only in the one answer that creates it (an API token, an
//! enrolment link, the secret and recovery codes of a one-time-code set-up) and is not repeated.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::session::{Identity, Provider, Role};

/// `GET /auth/options` (no sign-in needed): which ways of signing in this address offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AuthOptions {
    /// A passkey can be used on this address.
    pub passkey: bool,
    /// When not: why, in words for the person (a bare IP address, plain HTTP).
    pub passkey_problem: Option<String>,
    /// A password with a one-time code.
    pub code: bool,
    /// The old token link may still be used to sign in.
    pub token: bool,
    /// This visit can use its Tailscale account.
    pub tailscale: bool,
    /// An administrator requires a passkey or one-time codes: the token is refused.
    pub strong_required: bool,
}

/// `POST /auth/login` (no sign-in needed): the steps of signing in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum LoginRequest {
    /// Ask for a passkey challenge for this user name.
    PasskeyBegin { user: String },
    /// The browser's signed answer (`navigator.credentials.get`, base64url fields).
    PasskeyFinish {
        handle: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        credential: Value,
    },
    /// A password and a one-time code (or a recovery code in `code`).
    Code { user: String, password: String, code: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum LoginResponse {
    /// Give `options` to `navigator.credentials.get({ publicKey })`, then send the answer with `handle`.
    Challenge {
        handle: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        options: Value,
    },
    /// Signed in: the session cookie is set; `secret` must come back with every request (`X-Kora-Session`).
    SignedIn {
        identity: Identity,
        secret: String,
        /// A friendly note for the next screen (a recovery code was used; the token link should be replaced).
        notice: Option<String>,
    },
}

/// `POST /auth/enrol` (no sign-in needed; the link is the credential): the steps of the first set-up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum EnrolRequest {
    /// Who is this link for? Does not use it up.
    Check {
        link: String,
    },
    PasskeyBegin {
        link: String,
    },
    PasskeyFinish {
        link: String,
        handle: String,
        label: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        credential: Value,
    },
    /// Start the one-time-code set-up: a secret to scan.
    CodeBegin {
        link: String,
    },
    /// The password and a first code from the app; uses the link up.
    CodeFinish {
        link: String,
        password: String,
        code: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum EnrolResponse {
    Who {
        user: String,
        role: Role,
        passkey: bool,
        passkey_problem: Option<String>,
    },
    Challenge {
        handle: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        options: Value,
    },
    /// What the authenticator app needs: the secret as text, the `otpauth://` address, and the QR code as an SVG image.
    CodeSetup {
        secret: String,
        otpauth: String,
        qr_svg: String,
    },
    /// Set up and signed in. `recovery_codes` (one-time-code set-up only) are shown once.
    Done {
        identity: Identity,
        secret: String,
        recovery_codes: Vec<String>,
    },
}

/// One passkey of an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PasskeyView {
    pub id: i64,
    pub label: String,
    /// The name the passkey was registered for.
    pub rp_id: String,
    /// Unix seconds.
    pub created: i64,
    pub last_used: i64,
}

/// A signed-in browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceView {
    pub handle: String,
    pub device: String,
    pub address: String,
    pub signed_in: i64,
    pub last_seen: i64,
    /// The one making this request.
    pub current: bool,
}

/// `GET /auth/account` (signed-in remote user): their own credentials and devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountView {
    pub user: String,
    pub provider: Provider,
    pub role: Role,
    pub passkeys: Vec<PasskeyView>,
    /// A password and one-time codes are set up.
    pub code: bool,
    pub recovery_left: u32,
    /// The old token link still works for this user.
    pub legacy_token: bool,
    /// The user has a passkey or one-time codes (so the token link is not accepted for sign-in).
    pub strong: bool,
    /// Passkeys can be added from this address; when not, why.
    pub passkey_problem: Option<String>,
    /// Suggest setting up a passkey or one-time codes now (a token sign-in without them).
    pub upgrade_suggested: bool,
    pub devices: Vec<DeviceView>,
}

/// `POST /auth/account`: changes to your own sign-in (each needs a recent passkey or code check, except adding the first
/// credential).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum AccountChange {
    PasskeyBegin,
    PasskeyFinish {
        handle: String,
        label: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        credential: Value,
    },
    PasskeyRemove {
        id: i64,
    },
    CodeBegin,
    CodeFinish {
        password: String,
        code: String,
    },
    PasswordChange {
        current: String,
        password: String,
    },
    /// New recovery codes; the old ones stop working.
    RecoveryNew,
    DeviceRevoke {
        handle: String,
    },
    /// Sign out everywhere else.
    DevicesRevokeOthers,
    /// End the old token link of this user.
    TokenRevoke,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum AccountChanged {
    Done,
    Challenge {
        handle: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        options: Value,
    },
    CodeSetup {
        secret: String,
        otpauth: String,
        qr_svg: String,
    },
    /// Shown once.
    Recovery {
        codes: Vec<String>,
    },
}

/// `POST /auth/reauth`: a fresh check before the most sensitive actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum ReauthRequest {
    PasskeyBegin,
    PasskeyFinish {
        handle: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        credential: Value,
    },
    Code {
        password: String,
        code: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum ReauthResponse {
    Challenge {
        handle: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        options: Value,
    },
    /// Counts for `valid_for` seconds.
    Fresh { valid_for: u64 },
}

/// `GET /users/{name}/credentials` (on the computer that runs Kora).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UserCredentials {
    pub name: String,
    pub role: Role,
    pub passkeys: Vec<PasskeyView>,
    pub code: bool,
    pub recovery_left: u32,
    pub legacy_token: bool,
    pub signed_in_devices: u32,
}

/// `POST /users/{name}/reset`: what to take away, and give the person a new link to set up again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ResetWhat {
    Passkeys,
    Code,
    /// Everything, including the old token link.
    All,
    /// Only the old token link.
    LegacyToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResetRequest {
    pub what: ResetWhat,
}

/// `POST /users/{name}/reset` answer: their sessions have ended; the link lets them set up again. Shown once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResetDone {
    pub name: String,
    /// The secret part of the enrolment link; the page is `<panel address>/#enrol=<secret>`. `null` for a token-only reset.
    pub link: Option<String>,
    pub expires_in: u64,
}

/// One API token (never its secret).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ApiTokenView {
    pub id: i64,
    pub name: String,
    pub role: Role,
    pub created: i64,
    /// 0: never.
    pub expires: i64,
    /// 0: never used.
    pub last_used: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ApiTokensList {
    pub tokens: Vec<ApiTokenView>,
}

/// `POST /api-tokens`: a token for a script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ApiTokenRequest {
    pub name: String,
    pub role: Role,
    /// Valid for this many days; left out: never expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub days: Option<u32>,
}

/// Shown once. Send it as `Authorization: Bearer <token>`; it is never accepted in an address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NewApiToken {
    pub id: i64,
    pub name: String,
    pub role: Role,
    pub token: String,
}

/// `GET /tls`: the HTTPS state of the panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TlsStatus {
    /// `auto`, `provided`, `tailscale`, `self-signed` or `off`.
    pub mode: String,
    /// This panel's own connection is HTTPS.
    pub this_connection_secure: bool,
    pub active: bool,
    /// `provided`, `tailscale` or `self-signed` while a certificate is in use.
    pub source: Option<String>,
    pub names: Vec<String>,
    /// Unix seconds; 0 with no certificate.
    pub not_after: i64,
    pub days_left: Option<i64>,
    /// SHA-256 of the certificate in use, to compare with what a browser shows.
    pub fingerprint: Option<String>,
    /// For a self-signed certificate: the local authority's fingerprint, and whether it is available to download.
    pub ca_fingerprint: Option<String>,
    pub error: Option<String>,
    /// The certificate and key files of a provided certificate (paths, not contents).
    pub cert_path: Option<String>,
    /// A reverse proxy is trusted (`kora panel proxy trust`).
    pub proxy_trusted: bool,
}

/// `GET /tls/ca`: the local authority's certificate (public), to install in a browser or on a phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TlsCa {
    pub pem: String,
    pub fingerprint: String,
}
