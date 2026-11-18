//! Embeds `../../web/dist` (if present) as a generated `embedded.rs` of `include_bytes!` entries.
//! A missing or empty directory yields an empty list: the server then serves a built-in page.
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if let Ok(rel) = p.strip_prefix(base) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            out.push((rel, p.clone()));
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist = manifest.join("../../web/dist");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", dist.display());
    let mut files = Vec::new();
    if dist.is_dir() {
        let dist = dist.canonicalize().unwrap_or(dist);
        walk(&dist, &dist, &mut files);
    }
    let mut src = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for (rel, abs) in &files {
        let _ = writeln!(src, "    ({:?}, include_bytes!({:?})),", rel, abs.to_string_lossy());
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("embedded.rs");
    std::fs::write(out, src).unwrap();
}
