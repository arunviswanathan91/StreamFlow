//! Saving and loading gating trees in the StreamFLOW workspace (`.sfw`) JSON format.
//!
//! The on-disk shape is the one the Java app already writes, so existing workspaces open unchanged:
//! `"gates": { "<sample>": [ {id, parent_id?, name, type, x_channel, y_channel?, angle, xs, ys, ...} ] }`
//! with flat records and `parent_id` links (never nested JSON — tree depth is therefore unlimited, which
//! keeps the golden invariant: a population at any nesting depth can be opened).
//!
//! Additions, all optional keys that older readers ignore: `invert`, `edges` ("inclusive" | "half_open"; absent = inclusive, i.e. written by the Java app),
//! `type: "boolean"` with an `expr`. Loading is tolerant in the way the Java loader is (a missing parent
//! attaches to the root, same-named siblings keep the first — BUG-15 auto-heal) but never silent: every
//! repair or dropped record is returned as a warning.

use crate::{BoolExpr, EdgeRule, Gate, GateKind, GateTree, NodeId, NodeKind, Quad};
use sf_core::json::Json;
use sf_core::scale::ScaleSpec;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

/// Per-population presentation state persisted next to the gate (BUG-09 view, BUG-16 counts).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeMeta {
    pub view_x: Option<String>,
    pub view_y: Option<String>,
    pub view_x_scale: Option<String>,
    pub view_y_scale: Option<String>,
    pub lbl_dx: Option<f64>,
    pub lbl_dy: Option<f64>,
    /// Event count saved at the last evaluation (`None` = never computed).
    pub count: Option<i64>,
    pub parent_pct: Option<f64>,
    /// Original `type` string of an index-set population ("flowjo", "plugin", "embedding"), kept so a
    /// workspace round-trips through the Java app unchanged.
    pub type_hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SampleGates {
    pub tree: GateTree,
    pub meta: HashMap<NodeId, NodeMeta>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceError(pub String);

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "workspace: {}", self.0)
    }
}

impl std::error::Error for WorkspaceError {}

fn werr<T>(m: impl Into<String>) -> Result<T, WorkspaceError> {
    Err(WorkspaceError(m.into()))
}

// ----------------------------------------------------------------------------------------- writing

fn type_name(g: &Gate, meta: Option<&NodeMeta>) -> String {
    match &g.kind {
        GateKind::Rectangle => "rectangle".into(),
        GateKind::Interval => "interval".into(),
        GateKind::Ellipse => "ellipse".into(),
        GateKind::Polygon => "polygon".into(),
        GateKind::Quadrant(Quad::Q1) => "q1".into(),
        GateKind::Quadrant(Quad::Q2) => "q2".into(),
        GateKind::Quadrant(Quad::Q3) => "q3".into(),
        GateKind::Quadrant(Quad::Q4) => "q4".into(),
        GateKind::Indexed(_) => meta.and_then(|m| m.type_hint.clone()).unwrap_or_else(|| "flowjo".into()),
    }
}

fn nums(v: &[f64]) -> Json {
    Json::Arr(v.iter().map(|&x| Json::num(x)).collect())
}

fn expr_to_json(e: &BoolExpr, ids: &HashMap<NodeId, String>) -> Json {
    match e {
        BoolExpr::Ref(id) => {
            // A reference to a deleted population is written as an unresolvable id rather than lost;
            // loading reports it. (Evaluation already reports it as a warning.)
            let s = ids.get(id).cloned().unwrap_or_else(|| format!("deleted:{id}"));
            Json::obj().with("op", Json::Str("ref".into())).with("id", Json::Str(s))
        }
        BoolExpr::Not(x) => Json::obj().with("op", Json::Str("not".into())).with("arg", expr_to_json(x, ids)),
        BoolExpr::And(v) => Json::obj().with("op", Json::Str("and".into())).with("args", Json::Arr(v.iter().map(|x| expr_to_json(x, ids)).collect())),
        BoolExpr::Or(v) => Json::obj().with("op", Json::Str("or".into())).with("args", Json::Arr(v.iter().map(|x| expr_to_json(x, ids)).collect())),
    }
}

/// One sample's gating tree as the flat record array.
pub fn gates_to_json(tree: &GateTree, meta: &HashMap<NodeId, NodeMeta>) -> Json {
    let order = tree.self_and_descendants(GateTree::ROOT).expect("root always exists");
    let mut ids: HashMap<NodeId, String> = HashMap::new();
    for (k, &n) in order.iter().filter(|&&n| n != GateTree::ROOT).enumerate() {
        ids.insert(n, format!("G{}", k + 1));
    }
    let mut out = Vec::new();
    for &n in order.iter().filter(|&&n| n != GateTree::ROOT) {
        let m = meta.get(&n);
        let mut r = Json::obj().with("id", Json::Str(ids[&n].clone())).with("name", Json::Str(tree.name(n).unwrap().to_string()));
        if let Some(p) = tree.parent(n).unwrap() {
            if p != GateTree::ROOT {
                r = r.with("parent_id", Json::Str(ids[&p].clone()));
            }
        }
        match tree.kind(n).unwrap() {
            NodeKind::Root => unreachable!("the root is skipped"),
            NodeKind::Boolean(e) => {
                r = r.with("type", Json::Str("boolean".into())).with("expr", expr_to_json(e, &ids));
            }
            NodeKind::Gate(g) => {
                r = r.with("type", Json::Str(type_name(g, m))).with("angle", Json::num(g.angle)).with("xs", nums(&g.xs)).with("ys", nums(&g.ys));
                if let GateKind::Indexed(rows) = &g.kind {
                    r = r.with("plugin_indices", Json::Arr(rows.iter().map(|&v| Json::Num(v as f64)).collect()));
                } else {
                    r = r.with("x_channel", Json::Str(g.x_chan.clone()));
                    if let Some(y) = &g.y_chan {
                        r = r.with("y_channel", Json::Str(y.clone()));
                    }
                }
                if g.invert {
                    r = r.with("invert", Json::Bool(true));
                }
                if !g.x_scale.is_identity() {
                    r = r.with("x_scale", Json::Str(g.x_scale.to_text()));
                }
                if !g.y_scale.is_identity() {
                    r = r.with("y_scale", Json::Str(g.y_scale.to_text()));
                }
                if matches!(g.kind, GateKind::Rectangle | GateKind::Interval) {
                    let e = if g.edges == EdgeRule::HalfOpen { "half_open" } else { "inclusive" };
                    r = r.with("edges", Json::Str(e.into()));
                }
            }
        }
        if let Some(m) = m {
            let s = |k: &str, v: &Option<String>, r: Json| match v {
                Some(v) => r.with(k, Json::Str(v.clone())),
                None => r,
            };
            r = s("view_x", &m.view_x, r);
            r = s("view_y", &m.view_y, r);
            r = s("view_x_scale", &m.view_x_scale, r);
            r = s("view_y_scale", &m.view_y_scale, r);
            if m.lbl_dx.is_some() || m.lbl_dy.is_some() {
                r = r.with("lbl_dx", Json::num(m.lbl_dx.unwrap_or(0.0))).with("lbl_dy", Json::num(m.lbl_dy.unwrap_or(-4.0)));
            }
            if let Some(c) = m.count.filter(|&c| c >= 0) {
                r = r.with("count", Json::Num(c as f64));
            }
            if let Some(p) = m.parent_pct.filter(|p| p.is_finite()) {
                r = r.with("parent_pct", Json::Num(p));
            }
        }
        out.push(r);
    }
    Json::Arr(out)
}

/// `{ "<sample>": [records] }`, skipping samples with no gates (as the Java writer does).
pub fn workspace_to_json(samples: &BTreeMap<String, SampleGates>) -> Json {
    let mut o = Json::obj();
    for (name, s) in samples {
        if s.tree.len() > 1 {
            o = o.with(name, gates_to_json(&s.tree, &s.meta));
        }
    }
    o
}

// ----------------------------------------------------------------------------------------- reading

enum RawExpr {
    Ref(String),
    Not(Box<RawExpr>),
    And(Vec<RawExpr>),
    Or(Vec<RawExpr>),
}

impl RawExpr {
    fn parse(j: &Json) -> Result<RawExpr, String> {
        let op = j.get("op").and_then(Json::as_str).ok_or("expression has no 'op'")?;
        match op {
            "ref" => Ok(RawExpr::Ref(j.get("id").and_then(Json::as_str).ok_or("ref has no id")?.to_string())),
            "not" => Ok(RawExpr::Not(Box::new(RawExpr::parse(j.get("arg").ok_or("not has no 'arg'")?)?))),
            "and" | "or" => {
                let args = j.get("args").and_then(Json::as_arr).ok_or("and/or has no 'args'")?;
                let v = args.iter().map(RawExpr::parse).collect::<Result<Vec<_>, _>>()?;
                Ok(if op == "and" { RawExpr::And(v) } else { RawExpr::Or(v) })
            }
            other => Err(format!("unknown expression operator '{other}'")),
        }
    }

    fn collect_refs<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            RawExpr::Ref(s) => out.push(s),
            RawExpr::Not(e) => e.collect_refs(out),
            RawExpr::And(v) | RawExpr::Or(v) => v.iter().for_each(|e| e.collect_refs(out)),
        }
    }

    fn resolve(&self, ids: &HashMap<String, NodeId>) -> BoolExpr {
        match self {
            RawExpr::Ref(s) => BoolExpr::Ref(ids[s]),
            RawExpr::Not(e) => BoolExpr::Not(Box::new(e.resolve(ids))),
            RawExpr::And(v) => BoolExpr::And(v.iter().map(|e| e.resolve(ids)).collect()),
            RawExpr::Or(v) => BoolExpr::Or(v.iter().map(|e| e.resolve(ids)).collect()),
        }
    }
}

enum Body {
    Gate(Gate),
    Boolean(RawExpr),
}

struct Rec {
    id: String,
    parent: Option<String>,
    name: String,
    body: Body,
    meta: NodeMeta,
}

fn f64s(j: Option<&Json>) -> Option<Vec<f64>> {
    match j {
        None => Some(vec![]),
        Some(Json::Arr(a)) => a.iter().map(Json::as_f64).collect(),
        Some(_) => None,
    }
}

fn opt_str(j: &Json, k: &str) -> Option<String> {
    j.get(k).and_then(Json::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn parse_record(j: &Json) -> Result<Rec, String> {
    let id = j.get("id").and_then(Json::as_str).ok_or("record has no id")?.to_string();
    let name = j.get("name").and_then(Json::as_str).ok_or_else(|| format!("{id}: no name"))?.to_string();
    let ty = j.get("type").and_then(Json::as_str).ok_or_else(|| format!("'{name}': no type"))?;
    let parent = j.get("parent_id").and_then(Json::as_str).map(str::to_string);
    let mut meta = NodeMeta {
        view_x: opt_str(j, "view_x"),
        view_y: opt_str(j, "view_y"),
        view_x_scale: opt_str(j, "view_x_scale"),
        view_y_scale: opt_str(j, "view_y_scale"),
        lbl_dx: j.get("lbl_dx").and_then(Json::as_f64),
        lbl_dy: j.get("lbl_dy").and_then(Json::as_f64),
        count: j.get("count").and_then(Json::as_f64).filter(|&c| c >= 0.0 && c.fract() == 0.0).map(|c| c as i64),
        parent_pct: j.get("parent_pct").and_then(Json::as_f64),
        type_hint: None,
    };
    let body = match ty {
        "boolean" => Body::Boolean(RawExpr::parse(j.get("expr").ok_or_else(|| format!("'{name}': boolean gate has no expr"))?).map_err(|e| format!("'{name}': {e}"))?),
        "subsample" => return Err(format!("'{name}': random-subsample populations are not supported (they need the engine to redraw them)")),
        "flowjo" | "plugin" | "embedding" => {
            let raw = j.get("plugin_indices").and_then(Json::as_arr).ok_or_else(|| format!("'{name}': {ty} population has no saved event indices"))?;
            let mut rows = Vec::with_capacity(raw.len());
            for v in raw {
                match v.as_f64() {
                    Some(x) if x >= 0.0 && x.fract() == 0.0 && x <= u32::MAX as f64 => rows.push(x as u32),
                    _ => return Err(format!("'{name}': event index {v:?} is not a valid row number")),
                }
            }
            meta.type_hint = Some(ty.to_string());
            Body::Gate(Gate::indexed(rows))
        }
        _ => {
            let kind = match ty {
                "rectangle" => GateKind::Rectangle,
                "interval" | "line" => GateKind::Interval,
                "ellipse" => GateKind::Ellipse,
                "polygon" => GateKind::Polygon,
                "q1" => GateKind::Quadrant(Quad::Q1),
                "q2" => GateKind::Quadrant(Quad::Q2),
                "q3" => GateKind::Quadrant(Quad::Q3),
                "q4" => GateKind::Quadrant(Quad::Q4),
                other => return Err(format!("'{name}': unknown gate type '{other}'")),
            };
            let xs = f64s(j.get("xs")).ok_or_else(|| format!("'{name}': xs must be an array of numbers"))?;
            let ys = f64s(j.get("ys")).ok_or_else(|| format!("'{name}': ys must be an array of numbers"))?;
            let x = j.get("x_channel").and_then(Json::as_str).unwrap_or("");
            let y = j.get("y_channel").and_then(Json::as_str).filter(|s| !s.is_empty());
            let mut g = Gate::new(kind, x, y, xs, ys).map_err(|e| format!("'{name}': {e}"))?;
            g.angle = j.get("angle").and_then(Json::as_f64).filter(|a| a.is_finite()).unwrap_or(0.0);
            g.invert = j.get("invert").and_then(Json::as_bool).unwrap_or(false);
            // Java workspaces store raw-unit coordinates and no gate scale (identity); a scale key means the coordinates
            // are in that display space
            for (key, slot) in [("x_scale", &mut g.x_scale), ("y_scale", &mut g.y_scale)] {
                if let Some(s) = j.get(key).and_then(Json::as_str) {
                    *slot = ScaleSpec::parse(s).map_err(|e| format!("'{name}': {e}"))?.0;
                }
            }
            // a workspace written by the Java app has no "edges" key and was inclusive; keep its meaning
            g.edges = match j.get("edges").and_then(Json::as_str) {
                Some("half_open") => EdgeRule::HalfOpen,
                _ => EdgeRule::Inclusive,
            };
            Body::Gate(g)
        }
    };
    Ok(Rec { id, parent, name, body, meta })
}

/// Rebuild one sample's tree. Returns the tree plus a warning for every record that was repaired or
/// dropped; an `Err` means the document is not a record array at all.
pub fn gates_from_json(arr: &Json, root_name: &str) -> Result<(SampleGates, Vec<String>), WorkspaceError> {
    let items = match arr.as_arr() {
        Some(a) => a,
        None => return werr("a sample's gates must be an array"),
    };
    let mut warnings = Vec::new();
    let mut recs: Vec<Rec> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for item in items {
        match parse_record(item) {
            Ok(r) if !seen.insert(r.id.clone()) => warnings.push(format!("dropped '{}': duplicate id {}", r.name, r.id)),
            Ok(r) => recs.push(r),
            Err(e) => warnings.push(format!("dropped record: {e}")),
        }
    }
    let known: HashSet<String> = recs.iter().map(|r| r.id.clone()).collect();

    let mut sg = SampleGates { tree: GateTree::new(root_name), meta: HashMap::new() };
    let mut made: HashMap<String, NodeId> = HashMap::new();
    let mut dead: HashSet<String> = HashSet::new(); // dropped records; their subtrees go with them
    let mut pending: Vec<Rec> = recs;

    loop {
        let mut progress = false;
        let mut next = Vec::new();
        for r in pending {
            // where does it hang?
            let parent_node = match &r.parent {
                None => Some(GateTree::ROOT),
                Some(p) if dead.contains(p) => {
                    warnings.push(format!("dropped '{}': its parent was dropped", r.name));
                    dead.insert(r.id.clone());
                    progress = true;
                    continue;
                }
                Some(p) if !known.contains(p) => {
                    warnings.push(format!("'{}' named a missing parent ({p}); attached to the root", r.name));
                    Some(GateTree::ROOT)
                }
                Some(p) => made.get(p).copied(),
            };
            let Some(parent_node) = parent_node else {
                next.push(r); // parent not built yet
                continue;
            };
            let added = match &r.body {
                Body::Gate(g) => Some(sg.tree.add_gate(parent_node, &r.name, g.clone())),
                Body::Boolean(raw) => {
                    let mut refs = Vec::new();
                    raw.collect_refs(&mut refs);
                    if refs.iter().any(|s| dead.contains(*s) || !known.contains(*s)) {
                        warnings.push(format!("dropped Boolean gate '{}': it refers to a population that does not exist", r.name));
                        dead.insert(r.id.clone());
                        progress = true;
                        continue;
                    }
                    if refs.iter().all(|s| made.contains_key(*s)) {
                        Some(sg.tree.add_boolean(parent_node, &r.name, raw.resolve(&made)))
                    } else {
                        None // a referenced population is not built yet
                    }
                }
            };
            match added {
                None => next.push(r),
                Some(Ok(id)) => {
                    made.insert(r.id.clone(), id);
                    sg.meta.insert(id, r.meta);
                    progress = true;
                }
                Some(Err(e)) => {
                    warnings.push(format!("dropped '{}': {e}", r.name));
                    dead.insert(r.id.clone());
                    progress = true;
                }
            }
        }
        pending = next;
        if pending.is_empty() || !progress {
            break;
        }
    }
    for r in pending {
        warnings.push(format!("dropped '{}': unresolvable parent/reference (cycle?)", r.name));
    }
    Ok((sg, warnings))
}

/// Parse the `"gates"` object of a `.sfw` document: `{ sample: [records] }`.
pub fn workspace_from_json(gates: &Json, root_name: &str) -> Result<(BTreeMap<String, SampleGates>, Vec<String>), WorkspaceError> {
    let obj = match gates {
        Json::Obj(m) => m,
        Json::Null => return Ok((BTreeMap::new(), vec![])),
        _ => return werr("'gates' must be an object keyed by sample name"),
    };
    let (mut out, mut warnings) = (BTreeMap::new(), Vec::new());
    for (sample, arr) in obj {
        let (sg, w) = gates_from_json(arr, root_name)?;
        warnings.extend(w.into_iter().map(|m| format!("{sample}: {m}")));
        out.insert(sample.clone(), sg);
    }
    Ok((out, warnings))
}
