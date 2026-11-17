//! Generates the web panel's TypeScript API files from [`super::spec`] and the operations table.
//!
//! Output (both are checked in under `web/src/api/generated/` and marked "generated, do not edit"):
//!
//! * `types.ts`: every type reachable from a specification, declared once, in name order;
//! * `routes.ts`: `OpTypes` (per operation: params, query, body, response, job event/result types),
//!   the `ROUTES` table (method, path, whether it is a job) and `untypedOps`.
//!
//! Output is deterministic: operations in `Op::ALL` order, types sorted by name, `\n` line ends.
//! `crates/kora-web/tests/api_ts.rs` compares the checked-in files with [`generate`] and rewrites
//! them when `KORA_UPDATE_TS=1`.

use std::any::TypeId;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;

use ts_rs::{Config, TypeVisitor, TS};

use super::spec::{spec, Body, Resp, Ty};
use super::Op;

/// The TypeScript settings: 64-bit integers are plain numbers (timestamps, sizes; JSON cannot carry more).
pub fn config() -> Config {
    Config::new().with_large_int("number")
}

/// Collects the declaration of a type and of every type it uses, once each.
pub struct Collector {
    cfg: Config,
    seen: HashSet<TypeId>,
    decls: BTreeMap<String, (TypeId, String)>,
}

impl Collector {
    fn new() -> Self {
        Collector { cfg: config(), seen: HashSet::new(), decls: BTreeMap::new() }
    }

    pub fn add<T: TS + 'static + ?Sized>(&mut self) {
        if !self.seen.insert(TypeId::of::<T>()) {
            return;
        }
        if T::output_path().is_some() {
            let name = T::ident(&self.cfg);
            let mut decl = String::new();
            if let Some(d) = T::docs() {
                decl.push_str(d.trim_end());
                decl.push('\n');
            }
            decl.push_str("export ");
            decl.push_str(&pretty(&T::decl(&self.cfg)));
            if let Some((other, text)) = self.decls.get(&name) {
                // The same declaration reached through two paths (generic instantiations) is fine.
                assert!(*other == TypeId::of::<T>() || *text == decl, "two different Rust types are both called `{name}` in TypeScript: rename one");
            }
            self.decls.insert(name, (TypeId::of::<T>(), decl));
        }
        T::visit_dependencies(&mut Visit(self));
        T::visit_generics(&mut Visit(self));
    }
}

/// Lays out ts-rs's one-line declarations: one field per line, one object variant per line.
/// Only whitespace changes; strings and comments are copied as they are.
fn pretty(decl: &str) -> String {
    let chars: Vec<char> = decl.chars().collect();
    // A union with an object variant goes one variant per line.
    let break_union = {
        let (mut depth, mut parts, mut obj) = (0i32, 1, false);
        let mut i = 0;
        let mut in_str: Option<char> = None;
        while i < chars.len() {
            let ch = chars[i];
            match in_str {
                Some(q) => {
                    if ch == '\\' {
                        i += 1;
                    } else if ch == q {
                        in_str = None;
                    }
                }
                None => match ch {
                    '"' | '\'' => in_str = Some(ch),
                    '{' | '<' | '[' | '(' => {
                        if depth == 0 && ch == '{' {
                            obj = true;
                        }
                        depth += 1;
                    }
                    '}' | '>' | ']' | ')' => depth -= 1,
                    '|' if depth == 0 => parts += 1,
                    _ => {}
                },
            }
            i += 1;
        }
        parts > 1 && obj
    };
    let mut out = String::new();
    let base = usize::from(break_union);
    let (mut indent, mut brace, mut other) = (base, 0i32, 0i32);
    let mut i = 0;
    let mut in_str: Option<char> = None;
    let nl = |out: &mut String, indent: usize| {
        while out.ends_with([' ', '\n']) {
            out.pop();
        }
        out.push('\n');
        out.push_str(&"  ".repeat(indent));
    };
    let mut at_line_start = false;
    while i < chars.len() {
        let ch = chars[i];
        if let Some(q) = in_str {
            out.push(ch);
            if ch == '\\' && i + 1 < chars.len() {
                i += 1;
                out.push(chars[i]);
            } else if ch == q {
                in_str = None;
            }
            i += 1;
            continue;
        }
        if ch == '/' && chars.get(i + 1) == Some(&'*') {
            let end = decl[chars[..i].iter().map(|c| c.len_utf8()).sum::<usize>()..].find("*/").map(|e| e + 2).unwrap_or(0);
            let start_b: usize = chars[..i].iter().map(|c| c.len_utf8()).sum();
            let comment = &decl[start_b..start_b + end];
            let n_chars = comment.chars().count();
            if !at_line_start {
                nl(&mut out, indent);
            }
            for (k, line) in comment.lines().enumerate() {
                if k > 0 {
                    out.push('\n');
                    out.push_str(&"  ".repeat(indent));
                    out.push(' ');
                }
                out.push_str(line.trim());
            }
            nl(&mut out, indent);
            at_line_start = true;
            i += n_chars;
            continue;
        }
        match ch {
            '"' | '\'' => {
                in_str = Some(ch);
                out.push(ch);
                at_line_start = false;
            }
            '{' => {
                let rest = chars[i + 1..].iter().collect::<String>();
                if rest.trim_start().starts_with('}') {
                    out.push_str("{}");
                    i += 1 + rest.find('}').unwrap() + 1;
                    at_line_start = false;
                    continue;
                }
                out.push('{');
                brace += 1;
                indent += 1;
                nl(&mut out, indent);
                at_line_start = true;
            }
            '}' => {
                brace -= 1;
                indent = indent.saturating_sub(1).max(base);
                nl(&mut out, indent);
                out.push('}');
                at_line_start = false;
            }
            '<' | '[' | '(' => {
                other += 1;
                out.push(ch);
                at_line_start = false;
            }
            '>' | ']' | ')' => {
                other -= 1;
                out.push(ch);
                at_line_start = false;
            }
            ',' if other == 0 && brace > 0 => {
                out.push(',');
                nl(&mut out, indent);
                at_line_start = true;
            }
            '|' if break_union && brace == 0 && other == 0 => {
                nl(&mut out, 1);
                out.push_str("| ");
                at_line_start = false;
                i += 1;
                while chars.get(i) == Some(&' ') {
                    i += 1;
                }
                continue;
            }
            ' ' | '\n' | '\t' if at_line_start => {}
            '\n' | '\t' => out.push(' '),
            _ => {
                out.push(ch);
                at_line_start = false;
            }
        }
        i += 1;
    }
    // `= \n  | {`: the first variant gets its bar too.
    let out = if break_union { out.replacen("= {", "=\n  | {", 1) } else { out };
    out.lines().map(str::trim_end).collect::<Vec<_>>().join("\n")
}

struct Visit<'a>(&'a mut Collector);
impl TypeVisitor for Visit<'_> {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        self.0.add::<T>();
    }
}

/// The two generated files and a summary of what is still untyped.
pub struct Generated {
    pub types_ts: String,
    pub routes_ts: String,
    /// Names of operations whose body or response is still `Untyped`, with what is missing.
    pub untyped: Vec<String>,
}

const HEADER: &str = "// GENERATED by `cargo test -p kora-web --test api_ts` from the Rust API description\n// (crates/kora-core/src/api/{ops,spec,wire}.rs). DO NOT EDIT: change the Rust side and regenerate with\n//   KORA_UPDATE_TS=1 cargo test -p kora-web --test api_ts\n// CI fails when this file is out of date.\n";

fn ts_str(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// `/servers/{s}/logs/{file}` -> `["s", "file"]`.
pub fn path_params(path: &str) -> Vec<&str> {
    path.split('/').filter_map(|seg| seg.strip_prefix('{').and_then(|s| s.strip_suffix('}'))).collect()
}

fn ty_name(t: &Ty) -> String {
    t.name.clone()
}

pub fn generate() -> Generated {
    let mut c = Collector::new();
    // Shapes that no single operation names: the error envelope and the job record.
    c.add::<crate::problem::Problem>();
    c.add::<super::wire::ErrorEnvelope>();
    c.add::<super::wire::JobStarted>();
    c.add::<super::wire::JobRecord>();
    c.add::<super::wire::JobError>();
    c.add::<super::wire::JobFailedData>();
    let mut untyped = Vec::new();
    let mut ops = String::new();
    let mut routes = String::new();
    for op in Op::ALL {
        let (method, path) = op.route();
        let sp = spec(*op);
        let params = path_params(path);
        let params_ts = if params.is_empty() { "Record<string, never>".to_string() } else { format!("{{ {} }}", params.iter().map(|p| format!("{p}: string")).collect::<Vec<_>>().join("; ")) };
        let query_ts = sp.query.as_ref().map_or("undefined".to_string(), |t| {
            (t.collect)(&mut c);
            ty_name(t)
        });
        let body_ts = match &sp.body {
            Body::None => "undefined".to_string(),
            Body::Json(t) => {
                (t.collect)(&mut c);
                ty_name(t)
            }
            Body::Raw(_) => "Blob".to_string(),
            Body::Either(t, _) => {
                (t.collect)(&mut c);
                format!("{} | Blob", ty_name(t))
            }
            Body::Untyped => "Untyped".to_string(),
        };
        let resp_ts = match &sp.response {
            Resp::NoContent => "undefined".to_string(),
            Resp::Json(t) | Resp::Stream(_, t) => {
                (t.collect)(&mut c);
                ty_name(t)
            }
            Resp::Job => {
                c.add::<super::wire::JobStarted>();
                "JobStarted".to_string()
            }
            Resp::Untyped => "Untyped".to_string(),
        };
        let (ev_ts, res_ts) = match &sp.job {
            Some(j) => {
                let ev = j.events.as_ref().map_or("Untyped".to_string(), |t| {
                    (t.collect)(&mut c);
                    ty_name(t)
                });
                let re = j.result.as_ref().map_or("Untyped".to_string(), |t| {
                    (t.collect)(&mut c);
                    ty_name(t)
                });
                (ev, re)
            }
            None => ("undefined".into(), "undefined".into()),
        };
        let mut gaps = Vec::new();
        if !sp.body_typed() {
            gaps.push("request");
        }
        if !sp.response_typed() {
            gaps.push("response");
        }
        if let Some(j) = &sp.job {
            if j.events.is_none() {
                gaps.push("job events");
            }
            if j.result.is_none() {
                gaps.push("job result");
            }
        }
        if !gaps.is_empty() {
            untyped.push(format!("{}: {}", op.name(), gaps.join(", ")));
        }
        let stream = matches!(sp.response, Resp::Stream(..));
        let raw = match &sp.body {
            Body::Raw(t) | Body::Either(_, t) => ts_str(t),
            _ => "null".into(),
        };
        let _ = writeln!(
            ops,
            "  {name}: {{ method: '{m}'; path: {path_lit}; params: {params_ts}; query: {query_ts}; body: {body_ts}; response: {resp_ts}; events: {ev_ts}; result: {res_ts} }}",
            name = op.name(),
            m = method.as_str(),
            path_lit = ts_str(path),
        );
        let _ = writeln!(routes, "  {}: {{ method: '{}', path: {}, job: {}, stream: {stream}, raw: {raw} }},", op.name(), method.as_str(), ts_str(path), op.is_job());
    }

    let mut types_ts = String::from(HEADER);
    types_ts.push_str("\n/** Not described by the Rust side yet: the shape is unknown, so check it before use. */\nexport type Untyped = unknown\n\n");
    for (_, decl) in c.decls.values() {
        types_ts.push_str(decl);
        types_ts.push_str("\n\n");
    }
    while types_ts.ends_with("\n\n") {
        types_ts.pop();
    }

    // Only the types the table itself names (the rest are reached through them).
    let words: HashSet<&str> = ops.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')).collect();
    let used: Vec<&String> = c.decls.keys().filter(|k| words.contains(k.as_str())).collect();
    let mut routes_ts = String::from(HEADER);
    let names: Vec<&str> = std::iter::once("Untyped").chain(used.iter().map(|s| s.as_str())).collect();
    let _ = writeln!(routes_ts, "\nimport type {{ {} }} from './types'\n", names.join(", "));
    routes_ts.push_str("/** One entry per operation: what goes in and what comes out. `Untyped` = not described by the Rust side yet. */\nexport interface OpTypes {\n");
    routes_ts.push_str(&ops);
    routes_ts.push_str("}\n\nexport type OpName = keyof OpTypes\n\n");
    routes_ts.push_str("export interface RouteInfo {\n  method: 'GET' | 'POST' | 'PUT' | 'DELETE'\n  /** Relative to `/api/v1`; `{x}` is a path parameter. */\n  path: string\n  /** Answers `202 {job}`. */\n  job: boolean\n  /** `text/event-stream`. */\n  stream: boolean\n  /** The request body is a file of this content type. */\n  raw: string | null\n}\n\n");
    routes_ts.push_str("/** Every operation, in the order of `Op::ALL`. */\nexport const ROUTES: Record<OpName, RouteInfo> = {\n");
    routes_ts.push_str(&routes);
    routes_ts.push_str("}\n\n");
    routes_ts.push_str("/** Operations that still have an `Untyped` request, response or job type (a list to shrink). */\nexport const UNTYPED_OPS: readonly string[] = [\n");
    for u in &untyped {
        let _ = writeln!(routes_ts, "  {},", ts_str(u));
    }
    routes_ts.push_str("]\n");
    Generated { types_ts, routes_ts, untyped }
}
