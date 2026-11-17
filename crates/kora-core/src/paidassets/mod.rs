//! Paid (Cfx.re escrowed) assets: import the `.zip` files people download by hand from the Cfx.re
//! Portal (<https://portal.cfx.re/assets/granted-assets>). There is no official API and Kora does not
//! sign in for the user, so the flow is "download, then hand the zip to Kora".
//!
//! Escrow rules this module respects:
//! * every resource folder keeps its `fxmanifest.lua` and its hidden `.fxap` file;
//! * the folder name is never changed (the escrow is tied to it);
//! * zips are sometimes double-nested (`name/name/fxmanifest.lua`) or hold several resources;
//! * resources end up in `resources/[paid]/<name>` (a bracketed category folder);
//! * the server's licence key must belong to the Cfx.re account that owns the asset (see [`hints`]).
//!
//! * [`inspect`] reads a zip without extracting it and returns an [`ImportPlan`].
//! * [`import`] extracts into a staging folder inside the server folder, then swaps each resource into
//!   place. An existing copy is moved to `.kora/asset-backups/<name>-<millis>` first (the newest
//!   [`KEEP_BACKUPS`] are kept) and put back if anything fails. It adds `ensure <name>` to
//!   `server.cfg` and records the asset (source `"portal"`) in `kora-server.json`.
//! * [`remove`] moves an imported asset to the backups, drops its `ensure` line and its record.
//!
//! Symlinks, hard links and special entries in a zip are skipped, never created.

pub mod hints;

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::installer::registry::{now_unix, server_file_path, ResourceRecord, ServerFile};

pub use hints::{entitlement_hints, hints_for_server, EntitlementHint, HintKind};

/// Where bought assets are downloaded.
pub const PORTAL_URL: &str = "https://portal.cfx.re/assets/granted-assets";
/// `ResourceRecord::source` of an imported paid asset.
pub const SOURCE: &str = "portal";
/// The category folder (under `resources/`) paid assets go into.
pub const CATEGORY: &str = "[paid]";
/// Largest total size a zip may unpack to.
pub const MAX_TOTAL: u64 = 2 << 30;
/// Most entries a zip may have.
pub const MAX_ENTRIES: usize = 100_000;
/// Above this unpacked size a zip must not compress better than [`MAX_RATIO`] to one.
const BOMB_FLOOR: u64 = 32 << 20;
const MAX_RATIO: u64 = 100;
/// Backups kept per asset.
pub const KEEP_BACKUPS: usize = 3;

pub type Res<T> = Result<T, crate::problem::Problem>;

/// What a problem is about. `blocking` problems stop the import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "AssetProblemKind"))]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    NotAZip,
    NoResources,
    NoFxmanifest,
    MissingFxap,
    UnsafePath,
    SkippedEntry,
    TooBig,
    ZipBomb,
    Encrypted,
    NameTaken,
    BadName,
    Duplicate,
    NoFolder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "AssetProblem"))]
pub struct Problem {
    pub kind: ProblemKind,
    /// The resource it is about, when there is one.
    pub resource: Option<String>,
    pub message: String,
    pub blocking: bool,
}

/// One resource folder found in a zip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "FoundAsset"))]
pub struct FoundResource {
    /// The folder name (kept exactly; it is what `ensure` uses).
    pub name: String,
    /// The folder inside the zip (`""` when the zip itself is the resource).
    pub zip_path: String,
    pub has_fxmanifest: bool,
    pub has_fxap: bool,
    /// Looks escrowed: has a `.fxap`, mentions `escrow_ignore`, or holds encrypted Lua files.
    pub escrowed: bool,
    /// Unpacked size in bytes.
    pub size: u64,
    pub files: usize,
    /// From `fxmanifest.lua` (`version '1.2.3'`), when present.
    pub version: Option<String>,
    /// A paid asset with this name is already imported: importing replaces it (a backup is kept).
    pub replaces: bool,
}

/// What importing a zip would do. Nothing is written to make it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "AssetPlan"))]
pub struct ImportPlan {
    /// The zip's file name.
    pub zip_name: String,
    pub resources: Vec<FoundResource>,
    pub problems: Vec<Problem>,
    /// Unpacked size of everything that would be imported.
    pub total_size: u64,
    pub can_import: bool,
}

impl ImportPlan {
    /// Resources that would be imported (the ones with a manifest).
    pub fn importable(&self) -> impl Iterator<Item = &FoundResource> {
        self.resources.iter().filter(|r| r.has_fxmanifest)
    }
    fn refresh(&mut self) {
        self.can_import = self.importable().next().is_some() && !self.problems.iter().any(|p| p.blocking);
    }
    /// The blocking problems, one per line.
    pub fn blocking_text(&self) -> String {
        self.problems.iter().filter(|p| p.blocking).map(|p| p.message.clone()).collect::<Vec<_>>().join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportedAsset {
    pub name: String,
    pub version: Option<String>,
    /// An older copy was replaced; it was kept in the backups.
    pub replaced: bool,
    /// The backup folder (relative to the server folder) of the replaced copy.
    pub backup: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "AssetImportReport"))]
pub struct ImportReport {
    pub zip_name: String,
    pub imported: Vec<ImportedAsset>,
    /// Names `ensure` lines were added for.
    pub ensured: Vec<String>,
    /// Warnings from the plan and anything else worth saying.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportOpts {
    /// Allow replacing an asset that is already imported.
    pub replace: bool,
}

impl Default for ImportOpts {
    fn default() -> Self {
        Self { replace: true }
    }
}

/// An imported paid asset, as listed for a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PaidAsset {
    pub name: String,
    /// Relative to `resources/`.
    pub path: String,
    pub version: Option<String>,
    pub imported_unix: Option<u64>,
    /// Its folder is there.
    pub present: bool,
    /// Its `.fxap` file is there.
    pub has_fxap: bool,
    pub backups: usize,
}

/// The paid-assets screen of one server: what is imported and what the console says about it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AssetsView {
    pub assets: Vec<PaidAsset>,
    pub hints: Vec<EntitlementHint>,
}

/// [`AssetsView`] for the server at `dir`.
pub fn view(dir: &Path) -> Res<AssetsView> {
    Ok(AssetsView { assets: list(dir)?, hints: hints_for_server(dir) })
}

/// A zip in the Downloads folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DownloadZip {
    pub name: String,
    pub size: u64,
    pub modified_unix: u64,
}

// ------------------------------------------------------------------------------------------- names

/// A resource folder name that can be written to `server.cfg` as is (no category brackets).
pub fn valid_asset_name(n: &str) -> bool {
    !n.starts_with('[') && crate::api::valid_resource_name(n)
}

fn clean_name(name: &str) -> Result<Option<PathBuf>, ()> {
    let name = name.trim_end_matches('/');
    if name.is_empty() || name == "." {
        return Ok(None);
    }
    if name.contains(['\\', '\0', ':']) || name.starts_with('/') {
        return Err(());
    }
    let mut out = PathBuf::new();
    for c in Path::new(name).components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return Err(()),
        }
    }
    Ok(if out.as_os_str().is_empty() { None } else { Some(out) })
}

fn is_manifest(p: &Path) -> bool {
    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case("fxmanifest.lua") || n.eq_ignore_ascii_case("__resource.lua"))
}

fn junk(p: &Path) -> bool {
    p.components().next().is_some_and(|c| c.as_os_str() == "__MACOSX") || p.file_name().is_some_and(|n| n == ".DS_Store" || n == "Thumbs.db")
}

/// `version '1.2.3'` / `version "1.2.3"` from a manifest.
pub fn manifest_version(text: &str) -> Option<String> {
    for l in text.lines() {
        let l = l.trim();
        let Some(rest) = l.strip_prefix("version") else {
            continue;
        };
        let rest = rest.trim_start_matches([' ', '\t', '(']);
        let q = rest.chars().next()?;
        if q != '\'' && q != '"' {
            continue;
        }
        let body = &rest[1..];
        if let Some(end) = body.find(q) {
            let v = body[..end].trim();
            if !v.is_empty() && v.len() <= 64 && !v.chars().any(char::is_control) {
                return Some(v.to_string());
            }
        }
    }
    None
}

// ------------------------------------------------------------------------------------------- scan

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Dir,
}

#[derive(Debug, Clone)]
struct Entry {
    idx: usize,
    rel: PathBuf,
    kind: Kind,
    size: u64,
}

#[derive(Debug, Default)]
struct Scan {
    entries: Vec<Entry>,
    problems: Vec<Problem>,
}

fn problem(kind: ProblemKind, resource: Option<&str>, blocking: bool, message: impl Into<String>) -> Problem {
    Problem { kind, resource: resource.map(String::from), message: message.into(), blocking }
}

type Zip = zip::ZipArchive<io::BufReader<fs::File>>;

fn open_zip(path: &Path) -> Res<Zip> {
    let f = fs::File::open(path).map_err(|e| format!("Can't open {}: {e}", path.display()))?;
    zip::ZipArchive::new(io::BufReader::new(f)).map_err(|_| format!("{} is not a zip file Kora can read.", file_name(path)))
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// Unix file-type bits of a zip entry that is neither a regular file nor a folder (a link, device,
/// fifo, socket). Zips made on Windows have no Unix mode, which is fine.
fn special(mode: Option<u32>) -> bool {
    match mode.map(|m| m & 0o170000) {
        None | Some(0) | Some(0o100000) | Some(0o040000) => false,
        Some(_) => true,
    }
}

fn scan(z: &mut Zip) -> Scan {
    let mut s = Scan::default();
    if z.len() > MAX_ENTRIES {
        s.problems.push(problem(ProblemKind::TooBig, None, true, format!("The zip has more than {MAX_ENTRIES} files, which is far more than any resource needs.")));
        return s;
    }
    let mut seen = HashSet::new();
    let (mut skipped, mut unsafe_n) = (Vec::new(), 0usize);
    let (mut total, mut packed) = (0u64, 0u64);
    for i in 0..z.len() {
        let Ok(f) = z.by_index_raw(i) else {
            s.problems.push(problem(ProblemKind::NotAZip, None, true, "The zip is damaged. Download it again from the Cfx.re Portal."));
            return s;
        };
        let name = match f.name() {
            Ok(n) => n.into_owned(),
            Err(_) => {
                unsafe_n += 1;
                continue;
            }
        };
        let rel = match clean_name(&name) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(()) => {
                if unsafe_n < 3 {
                    s.problems.push(problem(ProblemKind::UnsafePath, None, true, format!("The zip contains an unsafe path ({name:?}) that would write outside its folder, so it was refused.")));
                }
                unsafe_n += 1;
                continue;
            }
        };
        if junk(&rel) {
            continue;
        }
        if f.is_symlink() || special(f.unix_mode()) {
            skipped.push(rel.display().to_string());
            continue;
        }
        if f.encrypted() {
            s.problems.push(problem(ProblemKind::Encrypted, None, true, "The zip is password-protected. Use the zip exactly as the Cfx.re Portal gives it."));
            return s;
        }
        if !seen.insert(rel.to_string_lossy().to_lowercase()) {
            s.problems.push(problem(ProblemKind::UnsafePath, None, true, format!("{} appears twice in the zip, so it was refused.", rel.display())));
            continue;
        }
        let kind = if f.is_dir() { Kind::Dir } else { Kind::File };
        let (size, csize) = if kind == Kind::File { (f.size(), f.compressed_size()) } else { (0, 0) };
        if kind == Kind::File && size > BOMB_FLOOR && size / csize.max(1) > MAX_RATIO * 10 {
            s.problems.push(problem(ProblemKind::ZipBomb, None, true, format!("{} would unpack to far more than its size in the zip; this looks like a zip bomb, so it was refused.", rel.display())));
            return s;
        }
        total = total.saturating_add(size);
        packed = packed.saturating_add(csize);
        s.entries.push(Entry { idx: i, rel, kind, size });
    }
    if unsafe_n > 3 {
        s.problems.push(problem(ProblemKind::UnsafePath, None, true, format!("{} more unsafe paths were found.", unsafe_n - 3)));
    }
    if !skipped.is_empty() {
        let shown: Vec<_> = skipped.iter().take(3).cloned().collect();
        s.problems.push(problem(
            ProblemKind::SkippedEntry,
            None,
            false,
            format!("{} link or special entr{} will be skipped ({}{}).", skipped.len(), if skipped.len() == 1 { "y" } else { "ies" }, shown.join(", "), if skipped.len() > 3 { ", ..." } else { "" }),
        ));
    }
    if total > MAX_TOTAL {
        s.problems.push(problem(ProblemKind::TooBig, None, true, format!("The zip would unpack to {}, more than the {} limit.", human(total), human(MAX_TOTAL))));
    } else if total > BOMB_FLOOR && total / packed.max(1) > MAX_RATIO {
        s.problems.push(problem(ProblemKind::ZipBomb, None, true, "The zip would unpack to far more than its own size; this looks like a zip bomb, so it was refused."));
    }
    s
}

/// `1.5 GB`-style size for messages.
pub fn human(n: u64) -> String {
    const U: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} bytes")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

fn read_text(z: &mut Zip, idx: usize, max: u64) -> Option<String> {
    let f = z.by_index(idx).ok()?;
    let mut buf = Vec::new();
    f.take(max).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn starts_with_fxap(z: &mut Zip, idx: usize) -> bool {
    let Ok(f) = z.by_index(idx) else { return false };
    let mut b = [0u8; 4];
    let mut r = f.take(4);
    r.read_exact(&mut b).is_ok() && &b == b"FXAP"
}

/// Resource roots: folders holding a manifest, outermost first; a manifest inside another root is
/// part of that resource.
fn roots_of(entries: &[Entry]) -> Vec<PathBuf> {
    let mut cands: Vec<PathBuf> = entries.iter().filter(|e| e.kind == Kind::File && is_manifest(&e.rel)).map(|e| e.rel.parent().map(Path::to_path_buf).unwrap_or_default()).collect();
    cands.sort_by_key(|p| p.components().count());
    cands.dedup();
    let mut roots: Vec<PathBuf> = Vec::new();
    for c in cands {
        if !roots.iter().any(|r| c.starts_with(r)) {
            roots.push(c);
        }
    }
    roots
}

fn stem(zip: &Path) -> String {
    zip.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Where a resource called `name` already is under `resources/` (FXServer looks in `resources/`
/// and nested `[category]` folders).
pub fn find_resource(resources: &Path, name: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
        let rd = fs::read_dir(dir).ok()?;
        let mut cats = Vec::new();
        for e in rd.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if !t.is_dir() {
                continue;
            }
            let n = e.file_name().to_string_lossy().into_owned();
            if n == name {
                return Some(e.path());
            }
            if n.starts_with('[') && n.ends_with(']') {
                cats.push(e.path());
            }
        }
        cats.sort();
        if depth == 0 {
            return None;
        }
        cats.iter().find_map(|c| walk(c, name, depth - 1))
    }
    walk(resources, name, 4)
}

/// What importing `zip` into the server at `server_dir` would do. Reads only.
pub fn inspect(server_dir: &Path, zip: &Path) -> ImportPlan {
    let mut plan = ImportPlan { zip_name: file_name(zip), ..Default::default() };
    let mut z = match open_zip(zip) {
        Ok(z) => z,
        Err(m) => {
            plan.problems.push(problem(ProblemKind::NotAZip, None, true, m));
            plan.refresh();
            return plan;
        }
    };
    let s = scan(&mut z);
    plan.problems = s.problems;
    let entries = s.entries;
    let roots = roots_of(&entries);
    let paid = records(server_dir).map(|r| r.paid()).unwrap_or_default();
    let resources_dir = server_dir.join("resources");
    let mut names_seen: HashSet<String> = HashSet::new();
    for root in &roots {
        let top = root.as_os_str().is_empty();
        let name = if top { stem(zip) } else { root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() };
        let mine: Vec<&Entry> = entries.iter().filter(|e| e.rel.starts_with(root) && e.rel != *root).collect();
        let rel_of = |e: &Entry| e.rel.strip_prefix(root).map(Path::to_path_buf).unwrap_or_default();
        let has_fxap = mine.iter().any(|e| e.kind == Kind::File && rel_of(e) == Path::new(".fxap"));
        let manifest = mine.iter().find(|e| e.kind == Kind::File && rel_of(e).components().count() == 1 && is_manifest(&e.rel)).map(|e| e.idx);
        let text = manifest.and_then(|i| read_text(&mut z, i, 256 * 1024)).unwrap_or_default();
        let version = manifest_version(&text);
        let mut escrowed = has_fxap || text.contains("escrow_ignore");
        if !escrowed {
            let luas: Vec<usize> =
                mine.iter().filter(|e| e.kind == Kind::File && e.rel.extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")) && !is_manifest(&e.rel)).take(40).map(|e| e.idx).collect();
            escrowed = luas.into_iter().any(|i| starts_with_fxap(&mut z, i));
        }
        let size: u64 = mine.iter().map(|e| e.size).sum();
        let files = mine.iter().filter(|e| e.kind == Kind::File).count();
        let n = name.as_str();
        if top {
            plan.problems.push(problem(
                ProblemKind::NoFolder,
                Some(n),
                false,
                format!(
                    "The zip has no folder around the resource, so it is named after the zip: {n}. If that is not the asset's real name, rename the zip to the name the creator uses before importing."
                ),
            ));
        }
        if !valid_asset_name(n) {
            plan.problems.push(problem(ProblemKind::BadName, Some(n), true, format!("'{n}' can't be used as a resource name (letters, digits, _ . and - only).")));
        } else if !names_seen.insert(n.to_lowercase()) {
            plan.problems.push(problem(ProblemKind::Duplicate, Some(n), true, format!("The zip holds two resources called {n}; FXServer can only load one.")));
        }
        if escrowed && !has_fxap {
            plan.problems.push(problem(
                ProblemKind::MissingFxap,
                Some(n),
                false,
                format!("{n} looks escrowed but has no hidden .fxap file, so it will not start. Download it again from the Cfx.re Portal and import the zip as it is (do not unpack and re-zip it)."),
            ));
        }
        let mut replaces = false;
        if valid_asset_name(n) {
            let recorded = paid.iter().any(|r| r.id == name);
            let ours = resources_dir.join(CATEGORY).join(n);
            match find_resource(&resources_dir, n) {
                Some(p) if p == ours => replaces = true,
                Some(p) => {
                    let rel = p.strip_prefix(server_dir).unwrap_or(&p).display().to_string();
                    plan.problems.push(problem(
                        ProblemKind::NameTaken,
                        Some(n),
                        true,
                        format!("There is already a resource called {n} in {rel}. Two resources with the same name can't both load: remove or move that one first, then import again."),
                    ));
                }
                None => replaces = recorded && ours.exists(),
            }
        }
        plan.resources.push(FoundResource { name, zip_path: zip_display(root), has_fxmanifest: true, has_fxap, escrowed, size, files, version, replaces });
    }
    // Top-level folders holding no resource at all.
    let mut tops: Vec<PathBuf> =
        entries.iter().filter(|e| e.rel.components().count() > 1 || e.kind == Kind::Dir).filter_map(|e| e.rel.components().next().map(|c| PathBuf::from(c.as_os_str()))).collect();
    tops.sort();
    tops.dedup();
    for t in tops {
        if roots.iter().any(|r| r.starts_with(&t) || r.as_os_str().is_empty()) {
            continue;
        }
        let name = t.display().to_string();
        let size = entries.iter().filter(|e| e.rel.starts_with(&t)).map(|e| e.size).sum();
        let files = entries.iter().filter(|e| e.rel.starts_with(&t) && e.kind == Kind::File).count();
        plan.problems.push(problem(
            ProblemKind::NoFxmanifest,
            Some(&name),
            roots.is_empty(),
            format!("{name} has no fxmanifest.lua, so it is not a resource{}.", if roots.is_empty() { "" } else { " and will be skipped" }),
        ));
        plan.resources.push(FoundResource { name, zip_path: zip_display(&t), has_fxmanifest: false, has_fxap: false, escrowed: false, size, files, version: None, replaces: false });
    }
    if roots.is_empty() && !plan.problems.iter().any(|p| p.blocking) {
        plan.problems.push(problem(ProblemKind::NoResources, None, true, "No resource was found in the zip (no fxmanifest.lua). Make sure you downloaded the asset itself from the Cfx.re Portal."));
    }
    plan.total_size = plan.importable().map(|r| r.size).sum();
    plan.refresh();
    plan
}

// ------------------------------------------------------------------------------------------- records

/// Where the paid-asset records live: `kora-server.json` for servers Kora installed, else
/// `.kora/paid-assets.json` (servers added from an existing folder).
pub enum Records {
    Server(Box<ServerFile>),
    Side { path: PathBuf, list: Vec<ResourceRecord> },
}

pub fn side_path(dir: &Path) -> PathBuf {
    dir.join(".kora").join("paid-assets.json")
}

/// Load the records of the server at `dir`.
pub fn records(dir: &Path) -> Res<Records> {
    if server_file_path(dir).exists() {
        return ServerFile::load(dir).map(|s| Records::Server(Box::new(s))).map_err(|e| format!("cannot read kora-server.json: {e}"));
    }
    let path = side_path(dir);
    let list = match fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    Ok(Records::Side { path, list })
}

impl Records {
    fn list(&self) -> &Vec<ResourceRecord> {
        match self {
            Records::Server(s) => &s.resources,
            Records::Side { list, .. } => list,
        }
    }
    fn list_mut(&mut self) -> &mut Vec<ResourceRecord> {
        match self {
            Records::Server(s) => &mut s.resources,
            Records::Side { list, .. } => list,
        }
    }
    /// The paid-asset records.
    pub fn paid(&self) -> Vec<ResourceRecord> {
        self.list().iter().filter(|r| r.is_portal()).cloned().collect()
    }
    fn upsert(&mut self, rec: ResourceRecord) {
        let l = self.list_mut();
        match l.iter_mut().find(|r| r.id == rec.id && r.is_portal()) {
            Some(r) => *r = rec,
            None => l.push(rec),
        }
    }
    fn remove(&mut self, name: &str) -> bool {
        let l = self.list_mut();
        let n = l.len();
        l.retain(|r| !(r.is_portal() && r.id == name));
        n != l.len()
    }
    fn save(&self) -> Res<()> {
        match self {
            Records::Server(s) => s.save().map_err(|e| format!("cannot write kora-server.json: {e}")),
            Records::Side { path, list } => {
                if let Some(p) = path.parent() {
                    fs::create_dir_all(p).map_err(|e| format!("cannot create {}: {e}", p.display()))?;
                }
                let tmp = path.with_extension("json.tmp");
                let body = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
                fs::write(&tmp, body).and_then(|()| fs::rename(&tmp, path)).map_err(|e| format!("cannot write {}: {e}", path.display()))
            }
        }
    }
}

fn backups_dir(dir: &Path) -> PathBuf {
    dir.join(".kora").join("asset-backups")
}

/// Backups of `name`, newest first.
fn backups_of(dir: &Path, name: &str) -> Vec<(u64, PathBuf)> {
    let prefix = format!("{name}-");
    let mut v: Vec<(u64, PathBuf)> = fs::read_dir(backups_dir(dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let ts: u64 = n.strip_prefix(&prefix)?.parse().ok()?;
            Some((ts, e.path()))
        })
        .collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.0));
    v
}

fn prune_backups(dir: &Path, name: &str) {
    for (_, p) in backups_of(dir, name).into_iter().skip(KEEP_BACKUPS) {
        let _ = fs::remove_dir_all(p);
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A fresh backup path for `name`.
fn new_backup_path(dir: &Path, name: &str) -> PathBuf {
    let mut ts = now_millis();
    loop {
        let p = backups_dir(dir).join(format!("{name}-{ts}"));
        if !p.exists() {
            return p;
        }
        ts += 1;
    }
}

/// The paid assets imported on the server at `dir`.
pub fn list(dir: &Path) -> Res<Vec<PaidAsset>> {
    let recs = records(dir)?;
    let mut out: Vec<PaidAsset> = recs
        .paid()
        .into_iter()
        .map(|r| {
            let p = dir.join("resources").join(&r.path);
            PaidAsset {
                present: p.join("fxmanifest.lua").is_file() || p.join("__resource.lua").is_file(),
                has_fxap: p.join(".fxap").is_file(),
                backups: backups_of(dir, &r.id).len(),
                name: r.id,
                path: r.path,
                version: r.version,
                imported_unix: r.imported_unix,
            }
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    Ok(out)
}

/// A folder inside the zip as people read it: always `/`, whatever the computer uses.
fn zip_display(p: &Path) -> String {
    p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

// ------------------------------------------------------------------------------------------- import

#[cfg(test)]
thread_local! {
    /// Tests: fail after this many resources were swapped into place.
    static FAIL_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

fn fail_point(done: usize) -> Res<()> {
    #[cfg(test)]
    if FAIL_AFTER.with(|f| f.get()) == Some(done) {
        return Err("simulated failure".into());
    }
    let _ = done;
    Ok(())
}

/// Removes the staging folder when dropped.
struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn extract_into(z: &mut Zip, entries: &[Entry], root: &Path, dest: &Path, budget: &mut u64) -> Res<()> {
    fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    for e in entries.iter().filter(|e| e.rel.starts_with(root) && e.rel != root) {
        let rel = e.rel.strip_prefix(root).map_err(|_| "bad path".to_string())?;
        let target = dest.join(rel);
        if e.kind == Kind::Dir {
            fs::create_dir_all(&target).map_err(|x| format!("cannot create {}: {x}", target.display()))?;
            continue;
        }
        if let Some(p) = target.parent() {
            fs::create_dir_all(p).map_err(|x| format!("cannot create {}: {x}", p.display()))?;
        }
        let f = z.by_index(e.idx).map_err(|x| format!("cannot read {} from the zip: {x}", e.rel.display()))?;
        let mut out = fs::OpenOptions::new().write(true).create_new(true).open(&target).map_err(|x| format!("cannot write {}: {x}", target.display()))?;
        let n = io::copy(&mut f.take(*budget + 1), &mut out).map_err(|x| format!("cannot unpack {}: {x}", e.rel.display()))?;
        if n > *budget {
            return Err(format!("The zip unpacks to more than {}; it was refused.", human(MAX_TOTAL)));
        }
        *budget -= n;
    }
    Ok(())
}

struct Swap {
    dest: PathBuf,
    backup: Option<PathBuf>,
}

fn undo(swaps: &[Swap]) {
    for s in swaps.iter().rev() {
        let _ = fs::remove_dir_all(&s.dest);
        if let Some(b) = &s.backup {
            let _ = fs::rename(b, &s.dest);
        }
    }
}

/// Import `zip` into the server at `dir`. `say` receives progress lines.
pub fn import(dir: &Path, zip: &Path, opts: &ImportOpts, say: &mut dyn FnMut(&str)) -> Res<ImportReport> {
    let plan = inspect(dir, zip);
    if !plan.can_import {
        let why = plan.blocking_text();
        return Err(if why.is_empty() { format!("Nothing to import from {}.", plan.zip_name) } else { why });
    }
    if !opts.replace {
        if let Some(r) = plan.importable().find(|r| r.replaces) {
            return Err(format!("{} is already imported. Choose to replace it to import this zip.", r.name));
        }
    }
    let mut report = ImportReport { zip_name: plan.zip_name.clone(), ..Default::default() };
    report.notes.extend(plan.problems.iter().filter(|p| !p.blocking).map(|p| p.message.clone()));

    let mut z = open_zip(zip)?;
    let s = scan(&mut z);
    if s.problems.iter().any(|p| p.blocking) {
        return Err("The zip changed while it was being imported. Try again.".into());
    }
    let roots = roots_of(&s.entries);
    let kora = dir.join(".kora");
    let staging = Staging(kora.join(format!("asset-import-{}", now_millis())));
    let _ = fs::remove_dir_all(&staging.0);
    fs::create_dir_all(&staging.0).map_err(|e| format!("cannot create {}: {e}", staging.0.display()))?;
    let mut budget = MAX_TOTAL;
    let mut staged: Vec<(&FoundResource, PathBuf)> = Vec::new();
    for r in plan.importable() {
        let root = roots.iter().find(|p| zip_display(p) == r.zip_path).ok_or("The zip changed while it was being imported. Try again.")?;
        say(&format!("Unpacking {}", r.name));
        let to = staging.0.join(&r.name);
        extract_into(&mut z, &s.entries, root, &to, &mut budget)?;
        if !(to.join("fxmanifest.lua").is_file() || to.join("__resource.lua").is_file()) {
            return Err(format!("{} has no fxmanifest.lua after unpacking.", r.name));
        }
        staged.push((r, to));
    }

    let category = dir.join("resources").join(CATEGORY);
    fs::create_dir_all(&category).map_err(|e| format!("cannot create {}: {e}", category.display()))?;
    let mut swaps: Vec<Swap> = Vec::new();
    let result = (|| -> Res<()> {
        for (r, from) in &staged {
            fail_point(swaps.len())?;
            let dest = category.join(&r.name);
            let mut backup = None;
            if dest.exists() {
                let b = new_backup_path(dir, &r.name);
                fs::create_dir_all(backups_dir(dir)).map_err(|e| format!("cannot create the backups folder: {e}"))?;
                fs::rename(&dest, &b).map_err(|e| format!("cannot move the old {} aside: {e}", r.name))?;
                backup = Some(b);
            }
            if let Err(e) = fs::rename(from, &dest) {
                if let Some(b) = &backup {
                    let _ = fs::rename(b, &dest);
                }
                return Err(format!("cannot move {} into place: {e}", r.name));
            }
            say(&format!("Placed {} in resources/{CATEGORY}", r.name));
            swaps.push(Swap { dest, backup });
        }
        fail_point(swaps.len())?;
        let mut recs = records(dir)?;
        let now = now_unix();
        for (r, _) in &staged {
            recs.upsert(ResourceRecord {
                id: r.name.clone(),
                path: format!("{CATEGORY}/{}", r.name),
                source: Some(SOURCE.into()),
                version: r.version.clone(),
                imported_unix: Some(now),
                ..Default::default()
            });
        }
        recs.save()
    })();
    if let Err(e) = result {
        undo(&swaps);
        return Err(format!("{e}. Nothing was changed: any replaced asset was put back."));
    }

    for ((r, _), s) in staged.iter().zip(&swaps) {
        report.imported.push(ImportedAsset {
            name: r.name.clone(),
            version: r.version.clone(),
            replaced: s.backup.is_some(),
            backup: s.backup.as_ref().map(|b| b.strip_prefix(dir).unwrap_or(b).display().to_string()),
        });
        prune_backups(dir, &r.name);
    }
    let names: Vec<String> = staged.iter().map(|(r, _)| r.name.clone()).collect();
    match crate::addresources::append_ensures_to_cfg(&dir.join("server.cfg"), &names) {
        Ok(added) => report.ensured = added,
        Err(e) => report.notes.push(e),
    }
    report.notes.push("Restart the server to load the new files.".into());
    Ok(report)
}

/// `cfg` without its `ensure <name>` lines, or `None` when there is none.
pub fn without_ensure(cfg: &str, name: &str) -> Option<String> {
    let is_it = |l: &str| {
        let mut w = l.split_whitespace();
        w.next() == Some("ensure") && w.next() == Some(name) && w.next().is_none_or(|x| x.starts_with('#'))
    };
    if !cfg.lines().any(is_it) {
        return None;
    }
    let mut out: String = cfg.lines().filter(|l| !is_it(l)).collect::<Vec<_>>().join("\n");
    if cfg.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    Some(out)
}

/// Take an imported asset off the server: its folder goes to the backups, its `ensure` line and its
/// record are removed. Returns notes to show.
pub fn remove(dir: &Path, name: &str) -> Res<Vec<String>> {
    if !valid_asset_name(name) {
        return Err(format!("'{name}' is not a resource name."));
    }
    let mut recs = records(dir)?;
    let Some(rec) = recs.paid().into_iter().find(|r| r.id == name) else {
        return Err(format!("There is no paid asset called {name} on this server (not found)."));
    };
    let mut notes = Vec::new();
    let folder = dir.join("resources").join(&rec.path);
    let mut backup = None;
    if folder.exists() {
        let b = new_backup_path(dir, name);
        fs::create_dir_all(backups_dir(dir)).map_err(|e| format!("cannot create the backups folder: {e}"))?;
        fs::rename(&folder, &b).map_err(|e| format!("cannot move {name} away: {e}"))?;
        backup = Some(b);
    }
    recs.remove(name);
    if let Err(e) = recs.save() {
        if let Some(b) = &backup {
            let _ = fs::rename(b, &folder);
        }
        return Err(e);
    }
    if let Some(b) = &backup {
        notes.push(format!("{name} was moved to {} (kept as a backup).", b.strip_prefix(dir).unwrap_or(b).display()));
        prune_backups(dir, name);
    }
    let cfg_path = dir.join("server.cfg");
    match fs::read_to_string(&cfg_path) {
        Ok(cfg) => {
            if let Some(new) = without_ensure(&cfg, name) {
                match fs::write(&cfg_path, new) {
                    Ok(()) => notes.push(format!("Removed 'ensure {name}' from server.cfg.")),
                    Err(e) => notes.push(format!("Could not write server.cfg ({e}); remove 'ensure {name}' yourself.")),
                }
            }
        }
        Err(e) => notes.push(format!("Could not read server.cfg ({e}); remove 'ensure {name}' yourself if it is there.")),
    }
    notes.push("Restart the server for this to take effect.".into());
    Ok(notes)
}

// ------------------------------------------------------------------------------------------- downloads

/// The user's Downloads folder (`$HOME/Downloads`, `%USERPROFILE%\Downloads`).
pub fn default_downloads_dir() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    let d = PathBuf::from(home).join("Downloads");
    d.is_dir().then_some(d)
}

/// Zips in `dir` (not in subfolders) changed within `max_age_secs` of `now`, newest first.
pub fn recent_zips(dir: &Path, now: u64, max_age_secs: u64) -> Vec<DownloadZip> {
    let mut v: Vec<DownloadZip> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            if !name.to_ascii_lowercase().ends_with(".zip") || name.starts_with('.') {
                return None;
            }
            // Not following links: only real files the user downloaded.
            let m = fs::symlink_metadata(e.path()).ok()?;
            if !m.is_file() {
                return None;
            }
            let modified = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            (now.saturating_sub(modified) <= max_age_secs).then_some(DownloadZip { name, size: m.len(), modified_unix: modified })
        })
        .collect();
    v.sort_by(|a, b| b.modified_unix.cmp(&a.modified_unix).then_with(|| a.name.cmp(&b.name)));
    v.truncate(50);
    v
}

/// `name` as a file directly inside `dir` (no folders, no `..`), if it is one of the recent zips.
pub fn resolve_download(dir: &Path, name: &str, now: u64, max_age_secs: u64) -> Res<PathBuf> {
    let plain = !name.is_empty() && Path::new(name).components().count() == 1 && matches!(Path::new(name).components().next(), Some(Component::Normal(_))) && !name.contains(['/', '\\']);
    if !plain || !recent_zips(dir, now, max_age_secs).iter().any(|z| z.name == name) {
        return Err(format!("{name} is not one of the recent zips in your Downloads folder."));
    }
    Ok(dir.join(name))
}

#[cfg(test)]
mod tests;
