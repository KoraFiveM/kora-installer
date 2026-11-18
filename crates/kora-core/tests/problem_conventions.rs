//! Keeps the "one error convention" from sliding back (see the `problem` module docs):
//!
//! * a `Problem` is never packed into a string: the packed marker and the helpers that used to
//!   pack and unpack it do not come back (the one place that still READS the old stored form is
//!   `store/jobs.rs`, for job rows written by v0.7.0);
//! * error codes are the constants of a `codes` module, never a string literal at the place a
//!   Problem is raised (a typo in a literal compiles; a typo in a constant does not).
//!
//! It reads the source tree of the workspace, so it needs no build of the other crates.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if !matches!(p.file_name().and_then(|n| n.to_str()), Some("target" | "node_modules" | "generated")) {
                rust_files(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Every Rust file of the main workspace's crates (not the desktop app's own workspace).
fn sources() -> Vec<(String, String)> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for c in ["kora-core", "kora-web", "kora-cli", "kora-shell"] {
        rust_files(&crates.join(c), &mut files);
    }
    files
        .into_iter()
        .filter(|p| !p.ends_with("problem_conventions.rs"))
        .map(|p| {
            let rel = p.strip_prefix(&crates).unwrap().to_string_lossy().replace('\\', "/");
            (rel, std::fs::read_to_string(&p).unwrap_or_default())
        })
        .collect()
}

/// The part of a file that is not test code (everything before its first `#[cfg(test)]`), or nothing
/// for a file in a `tests/` folder.
fn non_test_part<'a>(rel: &str, text: &'a str) -> &'a str {
    if rel.contains("/tests/") || rel.ends_with("/tests.rs") || rel.ends_with("tests_support.rs") {
        return "";
    }
    text.split("#[cfg(test)]").next().unwrap_or("")
}

#[test]
fn a_problem_is_never_packed_into_a_string_again() {
    let banned = [
        "kora-problem:",
        "Problem::decode",
        "Problem::is_encoded",
        "Problem::unpack",
        "problem::plain_text",
        "problem::message_of",
        "fn plain_text(",
        "fn message_of(",
        "impl From<Problem> for String",
        "problem_to_io",
        "problem_to_string",
    ];
    let mut found = Vec::new();
    for (rel, text) in sources() {
        // The one reader of the old stored form, and these module docs that name what was removed.
        if rel == "kora-core/src/store/jobs.rs" || rel == "kora-core/src/problem.rs" {
            continue;
        }
        for (n, line) in text.lines().enumerate() {
            for b in banned {
                if line.contains(b) {
                    found.push(format!("{rel}:{}: {b}", n + 1));
                }
            }
        }
    }
    assert!(found.is_empty(), "the packed Problem form or one of its helpers is back:\n{}", found.join("\n"));
}

#[test]
fn the_packed_marker_lives_in_one_place_only() {
    let holders: Vec<String> = sources().into_iter().filter(|(_, t)| t.contains("kora-problem:")).map(|(r, _)| r).collect();
    assert_eq!(holders, vec!["kora-core/src/store/jobs.rs".to_string()], "only the compatibility reader of old job rows may know the marker");
}

#[test]
fn error_codes_are_constants_not_string_literals() {
    let mut found = Vec::new();
    for (rel, text) in sources() {
        let code = non_test_part(&rel, &text);
        let mut rest = code;
        let mut offset = 0;
        while let Some(i) = rest.find("Problem::new(") {
            let after = &rest[i + "Problem::new(".len()..];
            if after.trim_start().starts_with('"') {
                let line = code[..offset + i].matches('\n').count() + 1;
                found.push(format!("{rel}:{line}"));
            }
            offset += i + 1;
            rest = &rest[i + 1..];
        }
    }
    assert!(found.is_empty(), "use a constant from a `codes` module for the code of a Problem:\n{}", found.join("\n"));
}
