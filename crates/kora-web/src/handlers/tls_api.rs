//! HTTPS state for Settings and the System check: which certificate, until when, its fingerprint; renew now; the local
//! authority's certificate (public) to trust once. Changing the setting itself is `kora panel tls ...` on the computer
//! (a certificate swapped over the connection it protects is not something to offer remotely), and renewing is local-only.
use axum::extract::State;
use axum::{Extension, Json};
use kora_core::api::wire;

use crate::remote::Caller;
use crate::tls::certs;
use crate::tls::config::{TlsConfig, TlsMode};
use crate::tls::serve::Transport;
use crate::tls::{selfsigned, CertSource};
use crate::{blocking, ApiError, AppState};

fn mode_text(m: TlsMode) -> &'static str {
    match m {
        TlsMode::Auto => "auto",
        TlsMode::Provided => "provided",
        TlsMode::Tailscale => "tailscale",
        TlsMode::SelfSigned => "self-signed",
        TlsMode::Off => "off",
    }
}

fn source_text(s: CertSource) -> &'static str {
    match s {
        CertSource::Provided => "provided",
        CertSource::Tailscale => "tailscale",
        CertSource::SelfSigned => "self-signed",
    }
}

pub(crate) fn status_of(st: &AppState, secure: bool) -> wire::TlsStatus {
    let store = st.audit_home.as_ref().and_then(|h| kora_core::store::Store::open_existing(h).ok().flatten());
    let cfg = store.as_ref().and_then(|s| TlsConfig::load(s).ok()).unwrap_or_default();
    let ca_fingerprint = store.as_ref().and_then(|s| selfsigned::load_ca(s).ok().flatten()).and_then(|ca| certs::parse_chain(ca.cert_pem.as_bytes()).ok()).map(|c| certs::fingerprint(&c[0]));
    let info = st.tls.info();
    wire::TlsStatus {
        mode: mode_text(cfg.mode).into(),
        this_connection_secure: secure,
        active: info.is_some(),
        source: info.as_ref().map(|i| source_text(i.source).into()),
        names: info.as_ref().map(|i| i.names.clone()).unwrap_or_default(),
        not_after: info.as_ref().map_or(0, |i| i.not_after),
        days_left: info.as_ref().map(|i| certs::days_left(i.not_after, crate::tls::manager::now_unix())),
        fingerprint: info.as_ref().map(|i| i.fingerprint.clone()),
        ca_fingerprint,
        error: st.tls.last_error(),
        cert_path: cfg.cert_path,
        proxy_trusted: st.remote.trusted_proxy().is_some(),
    }
}

/// `GET /tls`.
pub async fn tls_status(State(st): State<AppState>, Extension(t): Extension<Transport>) -> Result<Json<wire::TlsStatus>, ApiError> {
    blocking(move || Ok(Json(status_of(&st, t.secure)))).await
}

/// `POST /tls/renew` (on the computer that runs Kora): fetch or make the certificate again now.
pub async fn tls_renew(State(st): State<AppState>, Extension(c): Extension<Caller>, Extension(t): Extension<Transport>) -> Result<Json<wire::TlsStatus>, ApiError> {
    if c.remote {
        return Err(ApiError::forbidden("This can only be done on the computer that runs Kora, not over remote access."));
    }
    blocking(move || {
        let (Some(home), Some(mgr)) = (st.audit_home.clone(), st.tls_manager.clone()) else {
            return Err(ApiError::bad_request("This panel is not set up for HTTPS."));
        };
        crate::tls::manager::force_renewal(&home).map_err(ApiError::internal)?;
        mgr.reload(true).map_err(ApiError::bad_request)?;
        Ok(Json(status_of(&st, t.secure)))
    })
    .await
}

/// `GET /tls/ca`: the local authority's certificate (public) and its fingerprint.
pub async fn tls_ca(State(st): State<AppState>) -> Result<Json<wire::TlsCa>, ApiError> {
    blocking(move || {
        let store =
            st.audit_home.as_ref().and_then(|h| kora_core::store::Store::open_existing(h).ok().flatten()).ok_or_else(|| ApiError::not_found("This panel has no local certificate authority."))?;
        let ca = selfsigned::load_ca(&store).map_err(ApiError::internal)?.ok_or_else(|| ApiError::not_found("There is no local certificate authority yet: run `kora panel tls self-signed`."))?;
        let chain = certs::parse_chain(ca.cert_pem.as_bytes()).map_err(ApiError::internal)?;
        Ok(Json(wire::TlsCa { fingerprint: certs::fingerprint(&chain[0]), pem: ca.cert_pem }))
    })
    .await
}
