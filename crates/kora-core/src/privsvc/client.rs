//! The client side of the administrator service for code in `kora-core` and the command line (the panel has its own
//! seam, `kora-web`'s `elevated::Broker`, which does the same for its handlers): is the service there and serving
//! this account, and one request to it.
//!
//! A test (or the front end's test double) replaces the real service through [`set_override`].

use std::sync::{Arc, Mutex};

pub use super::conn::{Answer, ClientError};
use super::problems::Availability;

/// The way to the service.
pub trait Caller: Send + Sync {
    fn availability(&self) -> Availability;
    fn call(&self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<Answer, ClientError>;
}

/// The real service: a Unix socket on Linux, a named pipe on Windows.
struct Real;

impl Caller for Real {
    fn availability(&self) -> Availability {
        #[cfg(unix)]
        let r = super::unixsvc::probe(std::path::Path::new(super::paths::LINUX_SOCKET));
        #[cfg(windows)]
        let r = super::winsvc::probe();
        #[cfg(not(any(unix, windows)))]
        let r: Result<String, ClientError> = Err(ClientError::Unavailable("no administrator service on this system".into()));
        match r {
            Ok(_) => Availability::Ready,
            Err(ClientError::Refused(why)) => Availability::NotAllowed(why),
            Err(_) => Availability::Missing,
        }
    }

    fn call(&self, verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<Answer, ClientError> {
        #[cfg(unix)]
        let r = super::unixsvc::call(std::path::Path::new(super::paths::LINUX_SOCKET), verb, args, env, on_line);
        #[cfg(windows)]
        let r = super::winsvc::call(verb, args, env, on_line);
        #[cfg(not(any(unix, windows)))]
        let r: Result<Answer, ClientError> = {
            let _ = (verb, args, env, &on_line);
            Err(ClientError::Unavailable("no administrator service on this system".into()))
        };
        r
    }
}

static OVERRIDE: Mutex<Option<Arc<dyn Caller>>> = Mutex::new(None);

/// Test seam: use this caller instead of the real service (`None` = the real one again).
pub fn set_override(c: Option<Arc<dyn Caller>>) {
    *OVERRIDE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = c;
}

fn caller() -> Arc<dyn Caller> {
    OVERRIDE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().unwrap_or_else(|| Arc::new(Real))
}

pub fn availability() -> Availability {
    caller().availability()
}

/// One request. `Err(ClientError::Refused(why))` is the service's own refusal, in words for the person.
pub fn call(verb: &str, args: &[String], env: &[(String, String)], on_line: &mut dyn FnMut(&str)) -> Result<Answer, ClientError> {
    caller().call(verb, args, env, on_line)
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A scripted service for tests: records every request, answers from a fixed result.
    pub struct Scripted {
        pub availability: Availability,
        pub reply: Result<Answer, ClientError>,
        pub seen: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl Scripted {
        pub fn ready(ok: bool) -> Arc<Scripted> {
            Arc::new(Scripted { availability: Availability::Ready, reply: Ok(Answer { ok, code: i32::from(!ok), message: None, version: None, data: None }), seen: Mutex::new(vec![]) })
        }
        pub fn missing() -> Arc<Scripted> {
            Arc::new(Scripted { availability: Availability::Missing, reply: Err(ClientError::Unavailable("not running".into())), seen: Mutex::new(vec![]) })
        }
        pub fn requests(&self) -> Vec<(String, Vec<String>)> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Caller for Scripted {
        fn availability(&self) -> Availability {
            self.availability.clone()
        }
        fn call(&self, verb: &str, args: &[String], _: &[(String, String)], _: &mut dyn FnMut(&str)) -> Result<Answer, ClientError> {
            self.seen.lock().unwrap().push((verb.to_string(), args.to_vec()));
            self.reply.clone()
        }
    }

    /// Tests that use the override run one at a time.
    pub static SERIAL: Mutex<()> = Mutex::new(());
}
