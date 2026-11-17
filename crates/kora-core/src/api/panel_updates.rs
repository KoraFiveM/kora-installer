//! Response shapes the panel's update, resource and settings screens read.
//!
//! The engine types keep their on-disk / CLI formats (the catalog is camelCase, skip reasons are
//! enums); these small functions turn them into the plain JSON the web UI expects.

use serde_json::{json, Value};

use crate::catalog::{Catalog, Resource};
use crate::service::ScheduleConfig;
use crate::update::manual::UpdateEvent;
use crate::update::{DeferReason, SkipReason, UpdateReport};

use super::UpdateBadge;

/// The badge as the UI reads it: `{kind, count?}`.
pub fn badge_json(b: UpdateBadge) -> Value {
    match b {
        UpdateBadge::UpToDate => json!({"kind": "up_to_date"}),
        UpdateBadge::Updates(n) => json!({"kind": "updates", "count": n}),
        UpdateBadge::NeedsAttention => json!({"kind": "needs_attention"}),
        UpdateBadge::NewRelease => json!({"kind": "new_release"}),
    }
}

/// A job event for one update progress event.
pub fn update_event_json(e: &UpdateEvent) -> Value {
    match e {
        UpdateEvent::Resource { id, state } => {
            json!({"type": "resource", "id": id, "state": state})
        }
        UpdateEvent::Snapshot(id) => json!({"type": "snapshot", "id": id}),
        UpdateEvent::Health(m) => json!({"type": "health", "message": m}),
        UpdateEvent::Progress(m) => json!({"type": "progress", "message": m}),
        UpdateEvent::Checked { done, total, id } => json!({"type": "checked", "done": done, "total": total, "id": id}),
        UpdateEvent::Planned(ids) => json!({"type": "planned", "ids": ids}),
        UpdateEvent::Done => json!({"type": "done"}),
    }
}

fn skip_text(r: &SkipReason) -> String {
    match r {
        SkipReason::LocalCommits => "It has changes of your own, so it was left alone.".into(),
        SkipReason::Dirty => "It has files you changed, so it was not touched.".into(),
        SkipReason::Diverged => "It has changes of your own and upstream has new ones, so it was not touched.".into(),
        SkipReason::Error(e) => e.clone(),
        SkipReason::OtherBranch(b) => format!("It is {}, so it was not updated. Go back to the normal version to update it again.", crate::git::branches::describe_other(b)),
    }
}

fn defer_text(r: &DeferReason) -> String {
    match r {
        DeferReason::PlayersOnline(1) => "1 player is online.".into(),
        DeferReason::PlayersOnline(n) => format!("{n} players are online."),
        DeferReason::PlayerCountUnknown => "The number of players could not be checked.".into(),
        DeferReason::OutsideWindow => "It is outside the quiet hours.".into(),
    }
}

/// An update report with reasons as plain text (`[id, text]` pairs), like the UI's `UpdateReport`.
pub fn report_json(r: &UpdateReport) -> Value {
    json!({
        "server": r.server,
        "updated": r.updated.iter().map(|u| json!({"id": u.id, "from": u.from, "to": u.to})).collect::<Vec<_>>(),
        "skipped": r.skipped.iter().map(|(id, why)| json!([id, skip_text(why)])).collect::<Vec<_>>(),
        "deferred": r.deferred.iter().map(|(id, why)| json!([id, defer_text(why)])).collect::<Vec<_>>(),
        "rolled_back": r.rolled_back,
        "sql_flagged": r.sql_flagged,
        "needs_attention": r.needs_attention,
        "forks_synced": r.forks_synced,
        "forks_left_alone": r.forks_left_alone,
        "not_attempted": r.not_attempted,
        "held": r.held,
        "notes": r.notes,
    })
}

fn resource_json(r: &Resource) -> Value {
    let mut v = json!({
        "id": r.id,
        "repository": r.repository,
        "path": r.path,
        "group": r.group,
        "kind": r.kind,
        "branch": r.branch,
        "versions": r.versions.iter().map(|x| json!({"version": x.version, "tag": x.tag, "commit": x.commit})).collect::<Vec<_>>(),
        "depends_on": r.depends_on,
    });
    if let Some(t) = &r.title {
        v["title"] = json!(t);
    }
    if let Some(l) = &r.latest {
        v["latest"] = json!(l);
    }
    if let Some(c) = &r.creator {
        v["creator"] = json!(c);
    }
    if r.dev_only {
        v["dev_only"] = json!(true);
    }
    v
}

/// The catalog in the UI's snake_case shape. With `dev = Some(is_development)` only the
/// resources that kind of server may be offered are kept (no internal ones); `None` keeps them all
/// (the install wizard reads the whole catalog).
pub fn catalog_json(c: &Catalog, dev: Option<bool>) -> Value {
    let keep = |r: &&Resource| match dev {
        Some(d) => r.offered_to(d),
        None => true,
    };
    json!({
        "schema_version": c.schema_version,
        "resources": c.resources.iter().filter(keep).map(resource_json).collect::<Vec<_>>(),
        "creators": c.creators,
    })
}

/// What the UI shows for a schedule: the config, a sentence about it, and the "run as" help.
pub fn schedule_state_json(cfg: Option<&ScheduleConfig>) -> Value {
    json!({
        "schedule": cfg,
        "description": cfg.map(crate::service::describe),
        "run_as_help": crate::service::RUN_AS_HELP,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_reasons_become_text() {
        let r = UpdateReport { server: "s".into(), skipped: vec![("a".into(), SkipReason::Dirty)], deferred: vec![("b".into(), DeferReason::PlayersOnline(2))], ..UpdateReport::default() };
        let v = report_json(&r);
        assert_eq!(v["skipped"][0][0], "a");
        assert!(v["skipped"][0][1].is_string());
        assert_eq!(v["deferred"][0][1], "2 players are online.");
    }

    #[test]
    fn catalog_uses_snake_case_and_hides_internal() {
        let c = Catalog::embedded().unwrap();
        let v = catalog_json(&c, Some(false));
        assert!(v["schema_version"].is_number());
        let list = v["resources"].as_array().unwrap();
        assert!(!list.is_empty());
        assert!(list.iter().all(|r| r["kind"] != "internal" && r.get("dependsOn").is_none() && r["depends_on"].is_array()));
    }
}
