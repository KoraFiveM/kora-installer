//! The panel's data as the shell reads it: the parts of `GET /servers`, the `servers` event, `GET /alerts` and
//! `GET /self-update` that the tray and the notifications need. Deliberately tolerant: a field the shell does not
//! know, or a value a newer panel added, never breaks it (unknown words become `Other`/`Unknown`).

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ServerStatus {
    Running,
    Stopped,
    /// The folder is gone.
    Missing,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct PortsRow {
    #[serde(default)]
    pub game: u16,
    #[serde(default)]
    pub txadmin: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ServerRow {
    pub name: String,
    #[serde(default)]
    pub status: ServerStatus,
    #[serde(default)]
    pub ports: PortsRow,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ServersList {
    #[serde(default)]
    pub servers: Vec<ServerRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    Stopped,
    Recovered,
    DiskLow,
    RamHigh,
    UpdateAttention,
    BackupFailed,
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Danger,
    Warning,
    Success,
    #[default]
    #[serde(other)]
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AlertRow {
    pub id: u64,
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub kind: AlertKind,
    #[serde(default)]
    pub tone: Tone,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub message: String,
    /// Seconds since the Unix epoch.
    #[serde(default)]
    pub raised: u64,
    #[serde(default)]
    pub resolved: Option<u64>,
    #[serde(default)]
    pub acked: bool,
}

impl AlertRow {
    /// Still a problem somebody has not dealt with.
    pub fn open(&self) -> bool {
        self.resolved.is_none() && !self.acked
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AlertsList {
    #[serde(default)]
    pub alerts: Vec<AlertRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SelfUpdate {
    #[serde(default)]
    pub current: String,
    #[serde(default)]
    pub latest: Option<String>,
    #[serde(default)]
    pub available: bool,
}

/// What the shell knows right now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    /// The panel answered recently.
    pub connected: bool,
    pub servers: Vec<ServerRow>,
    pub alerts: Vec<AlertRow>,
    /// The version Kora itself could be updated to.
    pub update: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_the_panels_json_and_ignores_what_it_does_not_know() {
        let l: ServersList = serde_json::from_str(
            r#"{"servers":[{"name":"a","dir":"/x","status":"running","ports":{"game":30120,"txadmin":40120},"badge":null,"new_field":1},{"name":"b","status":"paused-by-a-newer-panel"}]}"#,
        )
        .unwrap();
        assert_eq!(l.servers[0].status, ServerStatus::Running);
        assert_eq!(l.servers[0].ports.txadmin, 40120);
        assert_eq!(l.servers[1].status, ServerStatus::Unknown);
        let a: AlertsList = serde_json::from_str(
            r#"{"alerts":[{"id":3,"server":"a","kind":"disk_low","tone":"warning","title":"t","message":"m","raised":5,"resolved":null,"acked":false},{"id":4,"kind":"future_kind","tone":"purple"}]}"#,
        )
        .unwrap();
        assert_eq!((a.alerts[0].kind, a.alerts[0].tone, a.alerts[0].open()), (AlertKind::DiskLow, Tone::Warning, true));
        assert_eq!((a.alerts[1].kind, a.alerts[1].tone), (AlertKind::Other, Tone::Info));
        let u: SelfUpdate = serde_json::from_str(r#"{"current":"0.7.0","latest":"0.8.0","available":true,"update_available":true,"versions":[]}"#).unwrap();
        assert!(u.available);
    }
}
