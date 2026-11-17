//! The connection loop: plain HTTP and HTTPS on one port, SSE and WebSocket upgrades included.
//!
//! Each accepted connection is looked at once, before anything is read: a TLS handshake starts with the byte
//! `0x16`, HTTP starts with a letter. Then:
//!
//! | listener has a certificate | first byte | peer | what happens |
//! |---|---|---|---|
//! | yes | TLS | any | handshake, then HTTP over TLS ([`Transport::secure`] is true) |
//! | yes | HTTP | this computer | served as plain HTTP (the launch link keeps working) |
//! | yes | HTTP | another computer | a `307` to the same address over `https://` ([`rules::redirect_location`]) |
//! | no | TLS | any | closed (there is nothing to answer with) |
//! | no | HTTP | any | plain HTTP, as before |
//!
//! Every request gets the peer address (`ConnectInfo`) and a [`Transport`] extension that tells the guard
//! whether it came over TLS (cookies get `Secure`, an `Origin` must be `https://`). Header and handshake
//! reads have time limits, so a connection that never finishes costs one task, not a thread.
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ConnectInfo;
use axum::Router;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tower_service::Service;

use super::rules;
use super::TlsState;

/// How a request reached the panel. Absent (tests calling the router directly) means plain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Transport {
    pub secure: bool,
}

/// Judge a `Host` header for the redirect (so the redirect never points anywhere the panel would refuse).
pub type HostCheck = Arc<dyn Fn(&str) -> bool + Send + Sync>;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const HEADER_TIMEOUT: Duration = Duration::from_secs(30);

/// Serve `app` on `listener` until `shutdown` finishes. `tls: None` is plain HTTP only.
pub async fn serve(listener: TcpListener, app: Router, tls: Option<TlsState>, host_ok: HostCheck, shutdown: impl Future<Output = ()>) -> std::io::Result<()> {
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => return Ok(()),
            accepted = listener.accept() => {
                let (stream, peer) = match accepted {
                    Ok(v) => v,
                    Err(e) => {
                        // Out of file handles and the like: wait a moment instead of spinning.
                        if !matches!(e.kind(), std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::ConnectionReset) {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                        continue;
                    }
                };
                let (app, tls, host_ok) = (app.clone(), tls.clone(), host_ok.clone());
                tokio::spawn(async move { connection(stream, peer, app, tls, host_ok).await });
            }
        }
    }
}

async fn connection(stream: TcpStream, peer: SocketAddr, app: Router, tls: Option<TlsState>, host_ok: HostCheck) {
    let _ = stream.set_nodelay(true);
    let mut first = [0u8; 1];
    let peeked = tokio::time::timeout(HEADER_TIMEOUT, stream.peek(&mut first)).await;
    if !matches!(peeked, Ok(Ok(1))) {
        return;
    }
    let looks_tls = first[0] == 0x16;
    let have_cert = tls.as_ref().is_some_and(TlsState::active);
    match (have_cert, looks_tls) {
        (true, true) => {
            let acceptor = tls.expect("have_cert").acceptor();
            let Ok(Ok(s)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await else { return };
            http(TokioIo::new(s), peer, app, true).await;
        }
        (true, false) if !crate::remote::is_remote_peer(peer.ip()) => http(TokioIo::new(stream), peer, app, false).await,
        (true, false) => redirect(stream, host_ok).await,
        (false, true) => {}
        (false, false) => http(TokioIo::new(stream), peer, app, false).await,
    }
}

async fn http<I>(io: I, peer: SocketAddr, app: Router, secure: bool)
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let svc = service_fn(move |mut req: hyper::Request<Incoming>| {
        let mut app = app.clone();
        async move {
            req.extensions_mut().insert(ConnectInfo(peer));
            req.extensions_mut().insert(Transport { secure });
            let resp: Result<axum::response::Response, Infallible> = app.call(req).await;
            resp
        }
    });
    let mut b = hyper::server::conn::http1::Builder::new();
    b.timer(TokioTimer::new()).header_read_timeout(HEADER_TIMEOUT);
    let _ = b.serve_connection(io, svc).with_upgrades().await;
}

/// Answer a plain-HTTP request on the TLS port with a redirect (GET/HEAD) or an explanation (anything else).
async fn redirect(mut stream: TcpStream, host_ok: HostCheck) {
    let mut buf = vec![0u8; 8192];
    let mut n = 0;
    let read = async {
        loop {
            match stream.read(&mut buf[n..]).await {
                Ok(0) | Err(_) => return,
                Ok(k) => n += k,
            }
            if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") || n == buf.len() {
                return;
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(5), read).await.is_err() {
        return;
    }
    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
    let reply = redirect_reply(&head, &*host_ok);
    let _ = stream.write_all(reply.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// The raw answer for a plain-HTTP request head.
pub fn redirect_reply(head: &str, host_ok: &dyn Fn(&str) -> bool) -> String {
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split(' ');
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let host = lines.filter_map(|l| l.split_once(':')).find(|(k, _)| k.eq_ignore_ascii_case("host")).map(|(_, v)| v.trim()).unwrap_or("");
    let page = |status: &str, extra: &str, body: &str| {
        format!("HTTP/1.1 {status}\r\n{extra}Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len())
    };
    match rules::redirect_location(host, target).filter(|_| host_ok(host)) {
        Some(loc) if method == "GET" || method == "HEAD" => {
            let body = format!("<!doctype html><meta charset=utf-8><title>Kora Panel</title><p>This panel uses HTTPS. <a href=\"{loc}\">Open it securely</a>.</p>");
            // 307, not 301/308: browsers cache those, and HTTPS can be turned off again.
            page("307 Temporary Redirect", &format!("Location: {loc}\r\n"), if method == "HEAD" { "" } else { &body })
        }
        Some(_) => page("426 Upgrade Required", "Upgrade: TLS/1.2\r\n", "<!doctype html><meta charset=utf-8><p>This panel uses HTTPS. Use the https:// address.</p>"),
        None => page("400 Bad Request", "", "<!doctype html><meta charset=utf-8><p>This panel uses HTTPS. Open it with https:// and the name or address you used before.</p>"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::certs::testcerts;
    use crate::tls::{certs, CertInfo, CertSource};

    fn ok_host(h: &str) -> bool {
        h == "box.lan:47120"
    }

    #[test]
    fn plain_requests_are_redirected_safely() {
        let r = redirect_reply("GET /install?a=1 HTTP/1.1\r\nHost: box.lan:47120\r\n\r\n", &ok_host);
        assert!(r.starts_with("HTTP/1.1 307") && r.contains("Location: https://box.lan:47120/install?a=1\r\n"), "{r}");
        // A Host the panel does not answer to, or a header-injection attempt, is never echoed.
        for head in
            ["GET / HTTP/1.1\r\nHost: evil.example\r\n\r\n", "GET / HTTP/1.1\r\nHost: box.lan:47120/x\r\n\r\n", "GET //evil.example HTTP/1.1\r\nHost: box.lan:47120\r\n\r\n", "GET / HTTP/1.1\r\n\r\n"]
        {
            let r = redirect_reply(head, &ok_host);
            assert!(r.starts_with("HTTP/1.1 400") && !r.contains("Location:"), "{head}: {r}");
        }
        let r = redirect_reply("POST /api/v1/session HTTP/1.1\r\nHost: box.lan:47120\r\n\r\n", &ok_host);
        assert!(r.starts_with("HTTP/1.1 426") && !r.contains("Location:"), "a POST body is never sent on: {r}");
    }

    fn app() -> Router {
        Router::new().route(
            "/who",
            axum::routing::get(|axum::Extension(t): axum::Extension<Transport>, ConnectInfo(p): ConnectInfo<SocketAddr>| async move {
                format!("secure={} peer_is_loopback={}", t.secure, p.ip().is_loopback())
            }),
        )
    }

    fn client_config(ca_pem: &str) -> Arc<rustls::ClientConfig> {
        let mut roots = rustls::RootCertStore::empty();
        for c in certs::parse_chain(ca_pem.as_bytes()).unwrap() {
            roots.add(c).unwrap();
        }
        Arc::new(
            rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }

    async fn get_plain(addr: SocketAddr, path: &str) -> String {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: box.lan:47120\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out).await;
        out
    }

    #[tokio::test]
    async fn one_port_serves_tls_and_plain_and_hot_swaps_the_certificate() {
        // A CA and a leaf for localhost.
        let ca = crate::tls::selfsigned::new_ca("test").unwrap();
        let leaf = crate::tls::selfsigned::issue_leaf(&ca, &["localhost".into()]).unwrap();
        let state = TlsState::new();
        let chain = certs::parse_chain(leaf.cert_pem.as_bytes()).unwrap();
        let sum = certs::summarise(&chain[0]).unwrap();
        let ck = certs::certified_key(chain, &certs::parse_key(leaf.key_pem.as_bytes()).unwrap()).unwrap();
        let info = CertInfo { source: CertSource::SelfSigned, names: sum.names, not_before: sum.not_before, not_after: sum.not_after, fingerprint: sum.fingerprint, ca_fingerprint: None };
        state.install(ck, info);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let host_ok: HostCheck = Arc::new(ok_host);
        let task = tokio::spawn(serve(listener, app(), Some(state.clone()), host_ok, async move {
            let _ = stopped.await;
        }));

        // HTTPS from the trusted CA works and the handler sees secure=true.
        let connector = tokio_rustls::TlsConnector::from(client_config(&ca.cert_pem));
        let tcp = TcpStream::connect(addr).await.unwrap();
        let mut tls = connector.connect(rustls::pki_types::ServerName::try_from("localhost").unwrap(), tcp).await.unwrap();
        tls.write_all(b"GET /who HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
        let mut out = String::new();
        let _ = tls.read_to_string(&mut out).await;
        assert!(out.contains("secure=true"), "{out}");

        // A client that does not trust the CA is refused at the handshake (no silent downgrade).
        let (other_ca, _) = testcerts::make(&["x"], 1);
        let bad = tokio_rustls::TlsConnector::from(client_config(&other_ca));
        let tcp = TcpStream::connect(addr).await.unwrap();
        assert!(bad.connect(rustls::pki_types::ServerName::try_from("localhost").unwrap(), tcp).await.is_err());

        // A wrong name is refused too.
        let tcp = TcpStream::connect(addr).await.unwrap();
        assert!(connector.connect(rustls::pki_types::ServerName::try_from("other.example").unwrap(), tcp).await.is_err());

        // Plain HTTP from this computer (loopback peer) is served as plain.
        let plain = get_plain(addr, "/who").await;
        assert!(plain.contains("secure=false") && plain.contains("peer_is_loopback=true"), "{plain}");

        // Swap the certificate without restarting: a new leaf from the same CA is what the next client sees.
        let leaf2 = crate::tls::selfsigned::issue_leaf(&ca, &["localhost".into(), "box.lan".into()]).unwrap();
        let chain2 = certs::parse_chain(leaf2.cert_pem.as_bytes()).unwrap();
        let fp2 = certs::fingerprint(&chain2[0]);
        let ck2 = certs::certified_key(chain2.clone(), &certs::parse_key(leaf2.key_pem.as_bytes()).unwrap()).unwrap();
        let sum2 = certs::summarise(&chain2[0]).unwrap();
        state.install(ck2, CertInfo { source: CertSource::SelfSigned, names: sum2.names, not_before: sum2.not_before, not_after: sum2.not_after, fingerprint: fp2.clone(), ca_fingerprint: None });
        let tcp = TcpStream::connect(addr).await.unwrap();
        let tls = connector.connect(rustls::pki_types::ServerName::try_from("box.lan").unwrap(), tcp).await.unwrap();
        let seen = certs::fingerprint(&tls.get_ref().1.peer_certificates().unwrap()[0]);
        assert_eq!(seen, fp2, "the new certificate is served on the next connection");

        let _ = stop.send(());
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn without_a_certificate_plain_http_is_untouched_and_tls_is_closed() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(serve(listener, app(), Some(TlsState::new()), Arc::new(ok_host), async move {
            let _ = stopped.await;
        }));
        assert!(get_plain(addr, "/who").await.contains("secure=false"));
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(&[0x16, 3, 1, 0, 5, 1, 0, 0, 1, 0]).await.unwrap();
        let mut out = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await;
        assert!(out.is_empty(), "nothing is answered to a TLS hello when there is no certificate");
        let _ = stop.send(());
        task.await.unwrap().unwrap();
    }
}
