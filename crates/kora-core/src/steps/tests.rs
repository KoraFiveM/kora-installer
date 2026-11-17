use super::*;
use std::{cell::RefCell, rc::Rc};
use tempfile::tempdir;

type Log = Rc<RefCell<Vec<String>>>;

struct Fake {
    id: &'static str,
    done: bool,
    fail_once: bool,
    log: Log,
}

impl Step for Fake {
    fn id(&self) -> &str {
        self.id
    }
    fn description(&self) -> &str {
        "fake"
    }
    fn verify(&self) -> StepResult<bool> {
        Ok(self.done)
    }
    fn run(&mut self) -> StepResult<()> {
        self.log.borrow_mut().push(format!("run {}", self.id));
        if self.fail_once {
            self.fail_once = false;
            return Err(StepError::Failed("boom".into()));
        }
        self.done = true;
        Ok(())
    }
    fn rollback(&mut self) -> StepResult<()> {
        self.log.borrow_mut().push(format!("rollback {}", self.id));
        self.done = false;
        Ok(())
    }
}

fn fake(id: &'static str, fail_once: bool, log: &Log) -> Box<dyn Step> {
    Box::new(Fake { id, done: false, fail_once, log: log.clone() })
}

#[test]
fn state_roundtrip() {
    let d = tempdir().unwrap();
    let p = d.path().join("sub/state.json");
    let mut s = PlanState::default();
    s.set("a", StepStatus::Failed, Some("x".into()));
    s.set("b", StepStatus::Done, None);
    s.save(&p).unwrap();
    assert_eq!(PlanState::load_or_default(&p).unwrap(), s);
    assert!(std::fs::read_to_string(&p).unwrap().contains("\"failed\""));
}

#[test]
fn missing_state_is_default() {
    let d = tempdir().unwrap();
    let s = PlanState::load_or_default(&d.path().join("none.json")).unwrap();
    assert!(s.steps.is_empty());
}

#[test]
fn runs_in_order_and_persists() {
    let d = tempdir().unwrap();
    let p = d.path().join("s.json");
    let log: Log = Default::default();
    let mut steps = vec![fake("a", false, &log), fake("b", false, &log)];
    let st = run_plan(&mut steps, &p).unwrap();
    assert_eq!(*log.borrow(), ["run a", "run b"]);
    assert_eq!(st.status("b"), Some(StepStatus::Done));
    assert_eq!(PlanState::load_or_default(&p).unwrap(), st);
}

#[test]
fn failure_then_resume_skips_done_steps() {
    let d = tempdir().unwrap();
    let p = d.path().join("s.json");
    let log: Log = Default::default();
    let mut steps = vec![fake("a", false, &log), fake("b", true, &log), fake("c", false, &log)];
    assert!(run_plan(&mut steps, &p).is_err());
    let st = PlanState::load_or_default(&p).unwrap();
    assert_eq!(st.status("a"), Some(StepStatus::Done));
    assert_eq!(st.status("b"), Some(StepStatus::Failed));
    assert_eq!(st.status("c"), Some(StepStatus::Pending));
    log.borrow_mut().clear();
    let st = run_plan(&mut steps, &p).unwrap();
    assert_eq!(*log.borrow(), ["run b", "run c"]);
    assert_eq!(st.status("c"), Some(StepStatus::Done));
}

#[test]
fn rollback_reverses_started_steps_only() {
    let d = tempdir().unwrap();
    let p = d.path().join("s.json");
    let log: Log = Default::default();
    let mut steps = vec![fake("a", false, &log), fake("b", true, &log), fake("c", false, &log)];
    let _ = run_plan(&mut steps, &p);
    log.borrow_mut().clear();
    let st = rollback_plan(&mut steps, &p).unwrap();
    assert_eq!(*log.borrow(), ["rollback b", "rollback a"]);
    assert_eq!(st.status("c"), Some(StepStatus::Pending));
    assert_eq!(st.status("a"), Some(StepStatus::RolledBack));
}

/// A step that fails every time, until it is skipped.
fn always_fails(id: &'static str, log: &Log) -> Box<dyn Step> {
    struct F(&'static str, Log);
    impl Step for F {
        fn id(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "always fails"
        }
        fn verify(&self) -> StepResult<bool> {
            Ok(false)
        }
        fn run(&mut self) -> StepResult<()> {
            self.1.borrow_mut().push(format!("run {}", self.0));
            Err(StepError::Failed("still broken".into()))
        }
        fn rollback(&mut self) -> StepResult<()> {
            self.1.borrow_mut().push(format!("rollback {}", self.0));
            Ok(())
        }
    }
    Box::new(F(id, log.clone()))
}

#[test]
fn a_skipped_step_is_passed_over_and_the_rest_runs() {
    let d = tempdir().unwrap();
    let p = d.path().join("s.json");
    let log: Log = Default::default();
    let mut steps = vec![fake("a", false, &log), always_fails("b", &log), fake("c", false, &log)];
    assert!(run_plan(&mut steps, &p).is_err());
    // Retrying alone fails again at the same step and never reaches c.
    assert!(run_plan(&mut steps, &p).is_err());
    assert_eq!(*log.borrow(), ["run a", "run b", "run b"]);
    log.borrow_mut().clear();
    let st = mark_skipped(&p, "b").unwrap();
    assert_eq!(st.status("b"), Some(StepStatus::Skipped));
    let st = run_plan(&mut steps, &p).unwrap();
    assert_eq!(*log.borrow(), ["run c"]);
    assert_eq!((st.status("b"), st.status("c")), (Some(StepStatus::Skipped), Some(StepStatus::Done)));
    assert!(std::fs::read_to_string(&p).unwrap().contains("\"skipped\""));
    // It stays skipped on later runs, and rollback still undoes whatever part of it ran.
    log.borrow_mut().clear();
    run_plan(&mut steps, &p).unwrap();
    assert!(log.borrow().is_empty());
    rollback_plan(&mut steps, &p).unwrap();
    assert_eq!(*log.borrow(), ["rollback c", "rollback b", "rollback a"]);
}

#[test]
fn a_done_step_is_never_marked_skipped() {
    let d = tempdir().unwrap();
    let p = d.path().join("s.json");
    let log: Log = Default::default();
    let mut steps = vec![fake("a", false, &log)];
    run_plan(&mut steps, &p).unwrap();
    assert_eq!(mark_skipped(&p, "a").unwrap().status("a"), Some(StepStatus::Done));
}

#[test]
fn a_failed_step_keeps_a_packed_problem_and_shows_text() {
    let p = crate::problem::Problem::new("x", "Readable.").with_action(crate::problem::Action::Retry);
    let e = StepError::Failed(p.clone());
    assert_eq!(e.to_string(), "step failed: Readable.");
}
