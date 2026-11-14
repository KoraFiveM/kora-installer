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
