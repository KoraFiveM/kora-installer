//! What the person reads, and can do, when the administrator service cannot do something for them.
//! There is no permission prompt any more: either the service is there and serves this account (then
//! the work simply happens), or there is a clear way forward. Every problem offers choices, never a
//! dead end: install the service (one administrator approval, once), go on without the administrator
//! parts (installs), or the command to run in an administrator terminal.

use crate::problem::{codes, Action, Problem};

/// What the panel was trying to do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// "install the server" / "change the automatic updates" (after "Kora cannot ").
    pub what: String,
    /// The command to run in an administrator terminal instead ("Show me the command").
    pub command: Vec<String>,
    /// "Continue without the administrator parts" makes sense here (installs).
    pub can_skip_admin: bool,
}

/// Is the service there for this account?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Ready,
    /// Not installed, or not running.
    Missing,
    /// Running, but this account is not on its list (the reason it gave).
    NotAllowed(String),
}

fn with_choices(mut p: Problem, ctx: &Context, install: bool) -> Problem {
    if install {
        p.push_action(Action::InstallAdminService);
    }
    if ctx.can_skip_admin {
        p.push_action(Action::ContinueWithoutAdmin);
    }
    if !ctx.command.is_empty() {
        p.push_action(Action::ShowCommand { command: ctx.command.clone(), elevated: true });
    }
    p
}

/// The Problem for a service that is not usable. `message` is the handler's own sentence when it
/// has a better one than the generic text.
pub fn unavailable(a: &Availability, ctx: &Context, user: &str) -> Problem {
    let what = if ctx.what.is_empty() { "do this".to_string() } else { ctx.what.clone() };
    match a {
        Availability::Ready => Problem::new(codes::FAILED, "Kora's administrator service is ready."),
        Availability::Missing => with_choices(
            Problem::new(
                codes::NEEDS_ELEVATION,
                format!(
                    "Kora needs administrator rights to {what}, and its administrator service is not installed on this computer. Install it once (one administrator approval) and Kora can do this, and the next time, without asking again. Or run the command yourself as administrator."
                ),
            ),
            ctx,
            true,
        ),
        Availability::NotAllowed(why) => {
            let mut p = Problem::new(
                codes::SERVICE_NOT_ALLOWED,
                format!("Kora's administrator service is installed, but it is not turned on for your account, so Kora cannot {what}. An administrator can turn it on for you."),
            )
            .with_details(why.clone());
            p.push_action(Action::ShowCommand { command: vec!["kora".into(), "service".into(), "allow".into(), if user.is_empty() { "<your account>".into() } else { user.to_string() }], elevated: true });
            if ctx.can_skip_admin {
                p.push_action(Action::ContinueWithoutAdmin);
            }
            if !ctx.command.is_empty() {
                p.push_action(Action::ShowCommand { command: ctx.command.clone(), elevated: true });
            }
            p
        }
    }
}

/// The service went away in the middle (stopped, restarted for an update, crashed).
pub fn lost(details: &str, ctx: &Context) -> Problem {
    let what = if ctx.what.is_empty() { "do this".to_string() } else { ctx.what.clone() };
    with_choices(
        Problem::new(codes::SERVICE_LOST, format!("Kora's administrator service stopped before it finished, so Kora could not {what}. The step can be tried again.")).with_details(details),
        ctx,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(skip: bool) -> Context {
        Context { what: "install the server".into(), command: vec!["kora".into(), "install".into()], can_skip_admin: skip }
    }
    fn kinds(p: &Problem) -> Vec<&'static str> {
        p.actions
            .iter()
            .map(|a| match a {
                Action::InstallAdminService => "install_admin_service",
                Action::ContinueWithoutAdmin => "continue_without_admin",
                Action::ShowCommand { .. } => "show_command",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn a_missing_service_offers_installing_it_going_on_without_and_the_command() {
        let p = unavailable(&Availability::Missing, &ctx(true), "jo");
        assert_eq!(p.code, codes::NEEDS_ELEVATION);
        assert_eq!(kinds(&p), ["install_admin_service", "continue_without_admin", "show_command"]);
        assert!(p.message.contains("install the server") && p.message.contains("one administrator approval"));
        assert_eq!(kinds(&unavailable(&Availability::Missing, &ctx(false), "jo")), ["install_admin_service", "show_command"]);
    }

    #[test]
    fn an_account_that_is_not_allowed_is_told_who_can_fix_it() {
        let p = unavailable(&Availability::NotAllowed("not on the list".into()), &ctx(true), "jo");
        assert_eq!(p.code, codes::SERVICE_NOT_ALLOWED);
        assert!(matches!(&p.actions[0], Action::ShowCommand { command, elevated: true } if command == &["kora", "service", "allow", "jo"]));
        assert!(p.actions.contains(&Action::ContinueWithoutAdmin));
        assert!(!kinds(&p).contains(&"install_admin_service"), "it is installed already");
    }

    #[test]
    fn a_lost_service_can_be_retried_and_never_mentions_a_prompt() {
        let p = lost("the connection closed", &ctx(false));
        assert_eq!(p.code, codes::SERVICE_LOST);
        let all = format!("{} {:?}", p.message, p.details);
        assert!(!all.to_lowercase().contains("windows") && !all.to_lowercase().contains("prompt"));
    }
}
