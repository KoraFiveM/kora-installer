//! Response shapes the panel's update, resource and settings screens read.
//!
//! The engine types keep their on-disk / CLI formats (the catalog is camelCase, skip reasons are
//! enums); these small functions turn them into the plain JSON the web UI expects.

use serde_json::Value;

use super::wire;

use crate::catalog::{Catalog, Resource};
use crate::service::ScheduleConfig;
use crate::update::manual::UpdateEvent;
use crate::update::{DeferReason, SkipReason, UpdateReport};

use super::UpdateBadge;

/// The badge as the UI reads it: `{kind, count?}` ([`wire::Badge`]).
pub fn badge_json(b: UpdateBadge) -> Value {
    to_value(&wire::Badge::from(b))
}

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// A job event for one update progress event ([`wire::UpdateEvent`]).
pub fn update_event_json(e: &UpdateEvent) -> Value {
    to_value(&match e {
        UpdateEvent::Resource { id, state } => wire::UpdateEvent::Resource { id: id.clone(), state: state.clone() },
        UpdateEvent::Snapshot(id) => wire::UpdateEvent::Snapshot { id: id.clone() },
        UpdateEvent::Health(m) => wire::UpdateEvent::Health { message: m.clone() },
        UpdateEvent::Progress(m) => wire::UpdateEvent::Progress { message: m.clone() },
        UpdateEvent::Checked { done, total, id } => wire::UpdateEvent::Checked { done: *done as u32, total: *total as u32, id: id.clone() },
        UpdateEvent::Planned(ids) => wire::UpdateEvent::Planned { ids: ids.clone() },
        UpdateEvent::Done => wire::UpdateEvent::Done,
    })
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

/// An update report with reasons as plain text (`[id, text]` pairs), like the UI's `UpdateReport` ([`wire::UpdateReport`]).
pub fn report_json(r: &UpdateReport) -> Value {
    to_value(&wire::UpdateReport {
        server: r.server.clone(),
        updated: r.updated.iter().map(|u| wire::UpdatedResource { id: u.id.clone(), from: u.from.clone(), to: u.to.clone() }).collect(),
        skipped: r.skipped.iter().map(|(id, why)| (id.clone(), skip_text(why))).collect(),
        deferred: r.deferred.iter().map(|(id, why)| (id.clone(), defer_text(why))).collect(),
        rolled_back: r.rolled_back.clone(),
        sql_flagged: r.sql_flagged.clone(),
        needs_attention: r.needs_attention.clone(),
        forks_synced: r.forks_synced.clone(),
        forks_left_alone: r.forks_left_alone.clone(),
        not_attempted: r.not_attempted.clone(),
        held: r.held.clone(),
        notes: r.notes.clone(),
    })
}

fn resource_json(r: &Resource) -> wire::CatalogResource {
    wire::CatalogResource {
        id: r.id.clone(),
        title: r.title.clone(),
        description: r.description.clone(),
        repository: r.repository.clone(),
        path: r.path.clone(),
        group: r.group,
        kind: r.kind,
        branch: r.branch.clone(),
        latest: r.latest.clone(),
        versions: r.versions.iter().map(|x| wire::CatalogRelease { version: x.version.clone(), tag: x.tag.clone(), commit: x.commit.clone() }).collect(),
        depends_on: r.depends_on.clone(),
        creator: r.creator.clone(),
        dev_only: r.dev_only.then_some(true),
        default_on: r.default_on.then_some(true),
    }
}

/// The catalog in the UI's snake_case shape ([`wire::Catalog`]). With `dev = Some(is_development)` only the
/// resources that kind of server may be offered are kept (no internal ones); `None` keeps them all
/// (the install wizard reads the whole catalog).
pub fn catalog_json(c: &Catalog, dev: Option<bool>) -> Value {
    let keep = |r: &&Resource| match dev {
        Some(d) => r.offered_to(d),
        None => true,
    };
    to_value(&wire::Catalog { schema_version: c.schema_version, resources: c.resources.iter().filter(keep).map(resource_json).collect(), creators: c.creators.clone() })
}

/// What the UI shows for a schedule: the config, a sentence about it, and the "run as" help.
pub fn schedule_state_json(cfg: Option<&ScheduleConfig>, job_found: bool) -> Value {
    // "Off" only when it is off: a saved schedule whose job cannot be seen from here says so instead.
    let description = cfg.map(|c| match (c.enabled, job_found) {
        (true, false) => format!("{}. Saved and on, but Kora cannot see the scheduled job from here. Press Save schedule to set it up again.", crate::service::describe(c)),
        _ => crate::service::describe(c),
    });
    to_value(&wire::ScheduleState { schedule: cfg.cloned(), description, job_found, run_as_help: crate::service::RUN_AS_HELP.to_string() })
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

    /// The pickers show a sentence under each name; the catalog file has it, so the API must send it
    /// (the panel's TypeScript always declared `description`, the answer used to leave it out).
    #[test]
    fn catalog_carries_each_resources_description() {
        let c = Catalog::embedded().unwrap();
        let v = catalog_json(&c, None);
        let with: Vec<&Value> = v["resources"].as_array().unwrap().iter().filter(|r| r["description"].is_string()).collect();
        assert!(!with.is_empty(), "the bundled catalog has descriptions, none came through");
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
