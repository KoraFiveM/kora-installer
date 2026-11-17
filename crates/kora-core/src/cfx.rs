//! Turning a Cfx.re (FiveM) username into the account id txAdmin and the server use.
//!
//! The id is the Cfx.re forum user id: `https://forum.cfx.re/u/<name>.json` answers `{"user": {"id": N,
//! "username": "Name"}}`, which is the same lookup txAdmin itself makes when you add an admin by name.
//! The identifier that goes into `admins.json` and `permissions.cfg` is `fivem:<id>`.
//!
//! The only thing sent over the network is the username, to forum.cfx.re, and only when someone asks
//! for this lookup.

use std::time::Duration;

/// A resolved account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfxAccount {
    /// Numeric Cfx.re account id.
    pub id: u64,
    /// The name as Cfx.re spells it (capitalisation can differ from what was typed).
    pub username: String,
}

impl CfxAccount {
    /// `fivem:<id>`, the identifier used by txAdmin and `add_principal identifier.fivem:<id> ...`.
    pub fn identifier(&self) -> String {
        format!("fivem:{}", self.id)
    }
}

/// How the lookup reaches the forum (replaced in tests).
pub trait Fetch {
    /// GET `url`; `Ok(Some(body))` for 200, `Ok(None)` for 404, `Err(plain message)` otherwise.
    fn get(&self, url: &str) -> Result<Option<String>, String>;
}

/// The real forum, over HTTPS, with short timeouts and no redirects.
pub struct Forum;

impl Fetch for Forum {
    fn get(&self, url: &str) -> Result<Option<String>, String> {
        let agent = ureq::AgentBuilder::new().redirects(0).timeout_connect(Duration::from_secs(8)).timeout(Duration::from_secs(15)).user_agent("kora-installer").build();
        match agent.get(url).call() {
            Ok(r) => r.into_string().map(Some).map_err(|_| "The Cfx.re forum sent an answer that could not be read.".to_string()),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(ureq::Error::Status(429, _)) => Err("The Cfx.re forum asked us to slow down. Wait a minute and try again.".into()),
            Err(ureq::Error::Status(c, _)) => Err(format!("The Cfx.re forum answered with an error ({c}). Try again in a moment.")),
            Err(_) => Err("Could not reach forum.cfx.re. Check the internet connection and try again.".into()),
        }
    }
}

/// A Cfx.re username is 3 to 20 letters, digits, `_`, `.` or `-`. Anything else is rejected before any request.
pub fn valid_username(name: &str) -> bool {
    (3..=20).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Look `name` up on the Cfx.re forum.
pub fn lookup_with(fetch: &dyn Fetch, name: &str) -> Result<CfxAccount, String> {
    let name = name.trim().trim_start_matches('@');
    if !valid_username(name) {
        return Err("A Cfx.re username has 3 to 20 letters, numbers, dots, dashes or underscores. It is the name you see on forum.cfx.re.".into());
    }
    let url = format!("https://forum.cfx.re/u/{name}.json");
    let body = fetch.get(&url)?.ok_or_else(|| format!("There is no Cfx.re account called '{name}'. Check the spelling: it is the name on your forum.cfx.re profile."))?;
    parse(&body).ok_or_else(|| "The Cfx.re forum answer was not in the expected form.".to_string())
}

/// [`lookup_with`] against the real forum.
pub fn lookup(name: &str) -> Result<CfxAccount, String> {
    lookup_with(&Forum, name)
}

fn parse(body: &str) -> Option<CfxAccount> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let u = v.get("user")?;
    let id = u.get("id")?.as_u64().filter(|n| *n > 0)?;
    let username = u.get("username")?.as_str()?.to_string();
    valid_username(&username).then_some(CfxAccount { id, username })
}

/// Accepts `fivem:<id>` (already resolved) and returns the id.
pub fn parse_identifier(s: &str) -> Option<u64> {
    s.trim().strip_prefix("fivem:")?.parse().ok().filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Result<Option<String>, String>, std::sync::Mutex<Vec<String>>);
    impl Fetch for Fake {
        fn get(&self, url: &str) -> Result<Option<String>, String> {
            self.1.lock().unwrap().push(url.to_string());
            self.0.clone()
        }
    }
    fn fake(r: Result<Option<String>, String>) -> Fake {
        Fake(r, Default::default())
    }

    #[test]
    fn a_found_account_gives_its_id_and_identifier() {
        let f = fake(Ok(Some(r#"{"user":{"id":4465268,"username":"JSixty","name":null}}"#.into())));
        let a = lookup_with(&f, " @jsixty ").unwrap();
        assert_eq!((a.id, a.username.as_str(), a.identifier().as_str()), (4465268, "JSixty", "fivem:4465268"));
        assert_eq!(f.1.lock().unwrap().as_slice(), ["https://forum.cfx.re/u/jsixty.json"]);
    }

    #[test]
    fn unknown_names_and_bad_input_are_plain_errors_and_bad_input_makes_no_request() {
        assert!(lookup_with(&fake(Ok(None)), "nobodyhere").unwrap_err().contains("no Cfx.re account called 'nobodyhere'"));
        for bad in ["", "ab", "a b c", "../etc/passwd", "name/with/slash", "x?y=1", "way-too-long-name-for-cfx-account", "näme"] {
            let f = fake(Ok(None));
            assert!(lookup_with(&f, bad).is_err(), "{bad}");
            assert!(f.1.lock().unwrap().is_empty(), "no request for {bad:?}");
        }
    }

    #[test]
    fn network_trouble_and_odd_answers_are_reported_not_trusted() {
        assert_eq!(lookup_with(&fake(Err("offline".into())), "someone").unwrap_err(), "offline");
        for body in ["not json", "{}", r#"{"user":{"id":0,"username":"x_y"}}"#, r#"{"user":{"id":"7","username":"abc"}}"#, r#"{"user":{"id":7,"username":"bad name!"}}"#] {
            assert!(lookup_with(&fake(Ok(Some(body.into()))), "someone").is_err(), "{body}");
        }
    }

    #[test]
    fn identifiers_are_parsed_strictly() {
        assert_eq!(parse_identifier("fivem:123"), Some(123));
        for bad in ["fivem:0", "fivem:", "steam:1", "123", "fivem:1x"] {
            assert_eq!(parse_identifier(bad), None, "{bad}");
        }
    }
}
