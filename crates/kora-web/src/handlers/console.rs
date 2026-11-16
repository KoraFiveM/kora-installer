//! Live console, sending commands, enabling the console helper resource.
//!
//! Each `pub async fn` is mounted by `crate::routes` for the [`Op`] of the same name; the HTTP
//! method and path come from `Op::route`, so only the handler's extractors live here.
#![allow(clippy::unused_async)]

use std::convert::Infallible;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use kora_core::api::Status;
use kora_core::console;
use serde::Deserialize;
use serde_json::json;
use tokio_stream::wrappers::ReceiverStream;

use crate::{blocking, ApiError, AppState};

const POLL: Duration = Duration::from_millis(500);
/// Lines replayed when the stream opens.
const BACKLOG: usize = 200;
/// Most bytes read from the log in one poll.
const CHUNK: u64 = 256 * 1024;

/// Follows the console log like the manager: the newest of the console / server logs.
struct Follower {
    dir: PathBuf,
    source: Option<PathBuf>,
    pos: u64,
    partial: String,
}

impl Follower {
    /// New lines since the last call. On the first call, the last [`BACKLOG`] lines.
    fn poll(&mut self, first: bool) -> Vec<String> {
        let src = console::source(&self.dir);
        let Some(p) = src.clone() else { return vec![] };
        let len = std::fs::metadata(&p).map_or(0, |m| m.len());
        if first || src != self.source {
            // First sight of a file (or the log switched): show its end, then follow.
            self.source = src;
            self.pos = len;
            self.partial.clear();
            return kora_core::api::logs::tail(&p, BACKLOG).unwrap_or_default();
        }
        if len < self.pos {
            self.pos = 0; // truncated or rotated
            self.partial.clear();
        }
        if len == self.pos {
            return vec![];
        }
        let Ok(mut f) = std::fs::File::open(&p) else {
            return vec![];
        };
        if f.seek(SeekFrom::Start(self.pos)).is_err() {
            return vec![];
        }
        let mut buf = Vec::new();
        if f.take(CHUNK).read_to_end(&mut buf).is_err() {
            return vec![];
        }
        self.pos += buf.len() as u64;
        self.partial.push_str(&String::from_utf8_lossy(&buf));
        let Some(end) = self.partial.rfind('\n') else {
            return vec![];
        };
        let rest = self.partial.split_off(end + 1);
        let done = std::mem::replace(&mut self.partial, rest);
        done.lines().map(kora_core::api::logs::clean_line).collect()
    }
}

pub async fn console_stream(
    State(st): State<AppState>,
    Path(s): Path<String>,
) -> Result<Sse<ReceiverStream<Result<Event, Infallible>>>, ApiError> {
    let e = blocking(move || st.entry(&s)).await?;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);
    tokio::spawn(async move {
        let mut f = Some(Follower {
            dir: e.dir,
            source: None,
            pos: 0,
            partial: String::new(),
        });
        let mut first = true;
        loop {
            let mut fol = f.take().expect("follower");
            let was_first = first;
            let Ok((fol2, lines)) = tokio::task::spawn_blocking(move || {
                let l = fol.poll(was_first);
                (fol, l)
            })
            .await
            else {
                return;
            };
            f = Some(fol2);
            // Nothing to replay on a server with no log yet: stay "first" until a file appears.
            first = first && f.as_ref().is_some_and(|x| x.source.is_none());
            for line in lines {
                let ev = Event::default()
                    .event("line")
                    .data(json!({"line": line}).to_string());
                if tx.send(Ok(ev)).await.is_err() {
                    return;
                }
            }
            tokio::select! {
                () = tokio::time::sleep(POLL) => {}
                () = tx.closed() => return,
            }
        }
    });
    Ok(Sse::new(ReceiverStream::new(rx))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

#[derive(Deserialize)]
pub struct SendBody {
    command: String,
}

pub async fn console_send(
    State(st): State<AppState>,
    Path(s): Path<String>,
    Json(body): Json<SendBody>,
) -> Result<Response, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        // Same rules as the manager: control characters dropped, not empty, not too long. A line
        // break could otherwise smuggle in a second command, so reject it instead of silently joining.
        if body.command.contains(['\n', '\r']) {
            return Err(ApiError::bad_request("Send one command at a time (no line breaks)."));
        }
        let cmd = console::clean_command(&body.command).map_err(ApiError::bad_request)?;
        if st.backend().status(&e) != Status::Running {
            return Err(ApiError::conflict("The server is not running, so it cannot take commands."));
        }
        if !console::is_installed(&e.dir) {
            return Err(ApiError::conflict(
                "The live console is not switched on for this server yet. Turn it on first (it needs a restart).",
            ));
        }
        console::send(&e.dir, &cmd).map_err(ApiError::internal)?;
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

pub async fn console_enable(
    State(st): State<AppState>,
    Path(s): Path<String>,
) -> Result<Response, ApiError> {
    blocking(move || {
        let e = st.entry(&s)?;
        if !e.dir.is_dir() {
            return Err(ApiError::conflict("The folder for this server is missing."));
        }
        console::install(&e.dir).map_err(ApiError::core)?;
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}
