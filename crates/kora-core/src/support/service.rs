//! The support-bundle operations, written once over [`Backend`] (the trait's default methods call these).

use std::path::PathBuf;

use super::bundle::{self, cache};
use super::system::SystemMachine;
use crate::api::wire::{BundleListing, BundleSaved};
use crate::api::Backend;
use crate::problem::Problem;

type Res<T> = Result<T, Problem>;

/// The sentence shown wherever a bundle is offered.
pub const NEVER_UPLOADED: &str = "Kora never uploads this file anywhere. It is saved on this computer, and it leaves only if you send it yourself.";

/// Build a bundle, keep it, and say what is in it.
pub fn preview<B: Backend + ?Sized>(b: &mut B, server: Option<&str>) -> Res<BundleListing> {
    let built = bundle::build(&SystemMachine::new(b), server);
    Ok(listing(built))
}

pub fn listing(built: bundle::Bundle) -> BundleListing {
    let (files, omitted, total) = (built.files.clone(), built.omitted.clone(), built.total_bytes());
    let id = cache::put(built);
    BundleListing { id, files, omitted, total_bytes: total, folder: bundle::default_folder().display().to_string(), never_uploaded: NEVER_UPLOADED.into() }
}

fn gone() -> String {
    Problem::generic("That support bundle is no longer in memory (they are kept for 15 minutes). Prepare it again.").into()
}

pub fn file_text(id: &str, name: &str) -> Res<String> {
    let b = cache::get(id).ok_or_else(gone)?;
    b.text(name).map(str::to_string).ok_or_else(|| Problem::generic("That file is not in the bundle.").into())
}

pub fn save(id: &str) -> Res<BundleSaved> {
    let b = cache::get(id).ok_or_else(gone)?;
    let folder = bundle::default_folder();
    let path = bundle::save(&b, &folder).map_err(|e| String::from(Problem::generic(format!("The support bundle could not be saved in {}: {e}", folder.display()))))?;
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(BundleSaved { file_name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), path: path.display().to_string(), bytes, folder: folder.display().to_string() })
}

/// The short cleaned text for an issue report.
pub fn summary<B: Backend + ?Sized>(b: &mut B, server: Option<&str>) -> String {
    bundle::build(&SystemMachine::new(b), server).summary_text()
}

/// A saved bundle by file name (only names Kora made, only in its own folder).
pub fn saved_path(file: &str) -> Option<PathBuf> {
    let ok = file.starts_with("kora-support-") && file.ends_with(".zip") && file.len() < 80 && file.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.'));
    let p = bundle::default_folder().join(file);
    (ok && p.is_file()).then_some(p)
}
