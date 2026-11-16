//! Embedded UI (`web/dist`) with SPA fallback, or a built-in "not built" page.
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded.rs"));
}

const NOT_BUILT: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Kora Panel</title>\
<style>body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;\
background:#111315;color:#e6e8e3;font:16px/1.5 system-ui,sans-serif}main{max-width:34rem;padding:2rem}\
h1{color:#A8FF00;margin:0 0 .5rem}code{background:#1c1f22;padding:.15rem .4rem;border-radius:4px;\
color:#A8FF00}</style></head><body><main><h1>Kora Panel</h1>\
<p>The web UI was not built into this binary.</p>\
<p>Build it with <code>cd web &amp;&amp; npm ci --ignore-scripts &amp;&amp; npm run build</code> and rebuild kora.</p>\
</main></body></html>";

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

fn find(path: &str) -> Option<&'static [u8]> {
    embedded::FILES
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, b)| *b)
}

fn respond(path: &str, body: &'static [u8], cache: &'static str) -> Response {
    let mut r = (StatusCode::OK, body).into_response();
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(path)),
    );
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    r
}

/// `uri_path` starts with `/`. Unknown paths fall back to `index.html` (SPA).
pub fn serve(uri_path: &str) -> Response {
    let rel = uri_path.trim_start_matches('/');
    if !rel.is_empty() && !rel.split('/').any(|s| s == ".." || s.is_empty()) {
        if let Some(b) = find(rel) {
            let cache = if rel.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            return respond(rel, b, cache);
        }
        // A missing file under /assets/ is a real 404, not the SPA shell.
        if rel.starts_with("assets/") {
            return StatusCode::NOT_FOUND.into_response();
        }
    }
    match find("index.html") {
        Some(b) => respond("index.html", b, "no-cache"),
        None => respond("index.html", NOT_BUILT.as_bytes(), "no-cache"),
    }
}
