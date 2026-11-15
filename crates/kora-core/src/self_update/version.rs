//! Semantic version comparison (semver 2.0 precedence; build metadata ignored).
//! `0.2.1 < 0.2.2-rc1 < 0.2.2 < 0.2.10`.

use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<String>,
}

impl Version {
    /// Accepts `v0.2.2`, `0.2.2-rc.1`, `0.2.2+build`.
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix('v').or_else(|| s.strip_prefix('V')).unwrap_or(s);
        let s = s.split('+').next()?;
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let mut it = core.split('.');
        let mut num = || -> Option<u64> {
            let p = it.next()?;
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) || (p.len() > 1 && p.starts_with('0')) {
                return None;
            }
            p.parse().ok()
        };
        let (major, minor, patch) = (num()?, num()?, num()?);
        if it.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => vec![],
            Some(p) => {
                let ids: Vec<String> = p.split('.').map(str::to_string).collect();
                if ids.iter().any(|i| i.is_empty() || !i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
                    return None;
                }
                ids
            }
        };
        Some(Version { major, minor, patch, pre })
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

fn cmp_id(a: &str, b: &str) -> Ordering {
    let num = |s: &str| s.bytes().all(|c| c.is_ascii_digit());
    match (num(a), num(b)) {
        (true, true) => a.len().cmp(&b.len()).then_with(|| a.cmp(b)), // no leading zeros guaranteed? compare by magnitude
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.cmp(b),
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(o.major, o.minor, o.patch)).then_with(|| {
            match (self.pre.is_empty(), o.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&o.pre) {
                        let c = cmp_id(a, b);
                        if c != Ordering::Equal {
                            return c;
                        }
                    }
                    self.pre.len().cmp(&o.pre.len())
                }
            }
        })
    }
}
impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }
    #[test]
    fn ordering_follows_semver() {
        assert!(v("0.2.1") < v("0.2.2-rc1"));
        assert!(v("0.2.2-rc1") < v("0.2.2"));
        assert!(v("0.2.2-rc1") < v("0.2.2-rc2"));
        assert!(v("0.2.2-rc.2") < v("0.2.2-rc.10"));
        assert!(v("0.2.2-1") < v("0.2.2-rc"));
        assert!(v("0.2.2-rc") < v("0.2.2-rc.1"));
        assert!(v("0.2.9") < v("0.2.10"));
        assert!(v("v1.0.0") > v("0.99.99"));
        assert_eq!(v("0.2.2+a"), v("0.2.2+b"));
        assert_eq!(v("v0.2.2").cmp(&v("0.2.2")), Ordering::Equal);
    }
    #[test]
    fn parse_rejects_garbage() {
        for bad in ["", "1", "1.2", "1.2.3.4", "a.b.c", "01.2.3", "1.2.3-", "1.2.3-a..b", "latest"] {
            assert!(Version::parse(bad).is_none(), "{bad}");
        }
        assert_eq!(v("v0.2.2-rc1").to_string(), "0.2.2-rc1");
        assert!(v("0.2.2-rc1").is_prerelease());
    }
}
