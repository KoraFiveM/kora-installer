//! API-only data shapes (the engine types that cross the API derive serde where they live).
//!
//! Naming notes: `catalog::Catalog` serialises in camelCase and `installer::StepStatus` has no
//! `rename_all`; both are on-disk / contract formats and were left alone.

use serde::{Deserialize, Serialize};

use crate::notify::{self, NotifyConfig};

/// What the API returns for a webhook: never the URL itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookView {
    pub set: bool,
    /// `discord.com/api/webhooks/<id>/***`
    pub redacted: Option<String>,
}

impl From<&NotifyConfig> for WebhookView {
    fn from(c: &NotifyConfig) -> Self {
        match c.discord_webhook.as_deref().filter(|u| !u.trim().is_empty()) {
            Some(u) => WebhookView { set: true, redacted: Some(notify::redact(u)) },
            None => WebhookView { set: false, redacted: None },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_view_never_carries_the_secret() {
        let url = "https://discord.com/api/webhooks/123456/SECRETTOKEN";
        let v = WebhookView::from(&NotifyConfig { discord_webhook: Some(url.into()) });
        let json = serde_json::to_string(&v).unwrap();
        assert!(v.set && !json.contains("SECRETTOKEN"), "{json}");
        assert_eq!(WebhookView::from(&NotifyConfig::default()), WebhookView { set: false, redacted: None });
        let bad = serde_json::to_string(&WebhookView::from(&NotifyConfig { discord_webhook: Some("https://x/SECRETTOKEN".into()) })).unwrap();
        assert!(!bad.contains("SECRETTOKEN"));
    }

    #[test]
    fn dtos_round_trip_in_snake_case() {
        use crate::api::{Status, UpdateBadge};
        use crate::service::ControlAction;
        use crate::update::manual::{ApplyOpts, ChangeStatus, RestartPolicy, Scope};
        let j = |v: &dyn erased::Ser| v.to_json();
        assert_eq!(j(&Status::Running), "\"running\"");
        assert_eq!(j(&UpdateBadge::NeedsAttention), "\"needs_attention\"");
        assert_eq!(j(&ControlAction::Restart), "\"restart\"");
        assert_eq!(j(&ChangeStatus::UpToDate), "\"up_to_date\"");
        assert_eq!(j(&Scope::All), "\"all\"");
        let s: Scope = serde_json::from_str(r#"{"resources":["a","b"]}"#).unwrap();
        assert_eq!(s, Scope::Resources(vec!["a".into(), "b".into()]));
        let o: ApplyOpts = serde_json::from_str(r#"{"restart":"when_quiet","skip":["x"]}"#).unwrap();
        assert_eq!((o.restart, o.skip), (RestartPolicy::WhenQuiet, vec!["x".to_string()]));
    }

    mod erased {
        pub trait Ser {
            fn to_json(&self) -> String;
        }
        impl<T: serde::Serialize> Ser for T {
            fn to_json(&self) -> String {
                serde_json::to_string(self).unwrap()
            }
        }
    }
}
