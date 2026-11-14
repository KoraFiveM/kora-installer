//! Pure helpers for a Windows `PATH` string (semicolon separated, case-insensitive).

fn norm(s: &str) -> String {
    s.trim().trim_end_matches(['\\', '/']).to_ascii_lowercase()
}

/// Does `path` already list `dir`? (case-insensitive, ignores a trailing slash and surrounding quotes)
pub fn contains(path: &str, dir: &str) -> bool {
    let want = norm(dir);
    path.split(';').any(|p| norm(p.trim().trim_matches('"')) == want)
}

/// `path` with `dir` appended, or `None` when it is already there (idempotent).
pub fn add(path: &str, dir: &str) -> Option<String> {
    if contains(path, dir) {
        return None;
    }
    let base = path.trim_end_matches(';');
    Some(if base.is_empty() { dir.to_string() } else { format!("{base};{dir}") })
}

/// `path` without any entry equal to `dir` (every other entry is kept verbatim), or `None` when it was not listed.
pub fn remove(path: &str, dir: &str) -> Option<String> {
    if !contains(path, dir) {
        return None;
    }
    let want = norm(dir);
    let kept: Vec<&str> = path.split(';').filter(|p| norm(p.trim().trim_matches('"')) != want).collect();
    Some(kept.join(";"))
}

#[cfg(test)]
mod tests {
    use super::*;
    const K: &str = r"C:\Program Files\Kora";

    #[test]
    fn add_is_idempotent_and_case_insensitive() {
        let p = r"C:\Windows;%SystemRoot%\System32";
        let once = add(p, K).unwrap();
        assert_eq!(once, format!("{p};{K}"));
        assert_eq!(add(&once, K), None);
        assert_eq!(add(&once.to_uppercase(), K), None, "case-insensitive");
        assert_eq!(add(&format!("{p};{K}\\"), K), None, "trailing slash");
        assert_eq!(add(&format!("{p};\"{K}\""), K), None, "quoted entry");
    }

    #[test]
    fn add_handles_empty_and_trailing_semicolon() {
        assert_eq!(add("", K).unwrap(), K);
        assert_eq!(add(r"C:\a;", K).unwrap(), format!(r"C:\a;{K}"));
    }

    #[test]
    fn remove_only_takes_our_entry() {
        let p = format!(r"C:\a;{K};C:\Program Files\Kora Other;%SystemRoot%\x");
        assert_eq!(remove(&p, K).unwrap(), r"C:\a;C:\Program Files\Kora Other;%SystemRoot%\x");
        assert_eq!(remove(r"C:\a", K), None);
    }
}
