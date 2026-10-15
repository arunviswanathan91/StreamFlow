//! GatingML 2.0 export, so StreamFLOW gates can be opened in FlowKit, FlowJo, FCS Express, Cytobank, ...
//!
//! What is exportable is deliberately limited to what GatingML can say *exactly*:
//!
//! * rectangle, interval, polygon and ellipse gates on **untransformed (linear) axes**, nested with
//!   `parent_id` (GatingML 2.0's own hierarchy attribute);
//! * Boolean And/Or/Not gates whose parent is the root (GatingML Boolean gates reference gates by id and
//!   carry no parent of their own).
//!
//! Everything else — gates drawn on a logicle/arcsinh/log axis (their coordinates are in display space and
//! would need the transform exported with them), quadrants, index populations, inverted gates, nested
//! Booleans — is skipped and named in the returned warnings; nothing is exported approximately.
//! GatingML rectangles are half-open (`min <= v < max`); an `Inclusive` gate is exported as written but
//! flagged, because a consumer will treat its upper edge differently.

use crate::workspace::NodeMeta;
use crate::{BoolExpr, EdgeRule, Gate, GateKind, GateTree, NodeId, NodeKind};
use sf_core::xml::escape;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

pub struct Export {
    pub xml: String,
    /// `(population id in the file, population path)` for every exported gate, in file order.
    pub exported: Vec<(String, Vec<String>)>,
    pub warnings: Vec<String>,
}

fn is_linear(scale: &Option<String>) -> bool {
    match scale.as_deref() {
        None => true,
        Some(s) => s.eq_ignore_ascii_case("linear"),
    }
}

fn dim(out: &mut String, chan: &str, bounds: Option<(f64, f64)>) {
    match bounds {
        Some((lo, hi)) => {
            let _ = write!(out, "  <gating:dimension gating:compensation-ref=\"uncompensated\" gating:min=\"{lo}\" gating:max=\"{hi}\">");
        }
        None => out.push_str("  <gating:dimension gating:compensation-ref=\"uncompensated\">"),
    }
    let _ = writeln!(out, "<data-type:fcs-dimension data-type:name=\"{}\"/></gating:dimension>", escape(chan));
}

fn rng(v: &[f64]) -> (f64, f64) {
    (v.iter().cloned().fold(f64::INFINITY, f64::min), v.iter().cloned().fold(f64::NEG_INFINITY, f64::max))
}

fn gate_xml(out: &mut String, id: &str, parent: Option<&str>, g: &Gate) -> Result<(), String> {
    let pa = parent.map(|p| format!(" gating:parent_id=\"{}\"", escape(p))).unwrap_or_default();
    let y = || g.y_chan.as_deref().ok_or_else(|| "two-parameter gate without a y channel".to_string());
    match &g.kind {
        GateKind::Rectangle => {
            let _ = writeln!(out, "<gating:RectangleGate gating:id=\"{}\"{pa}>", escape(id));
            dim(out, &g.x_chan, Some(rng(&g.xs)));
            dim(out, y()?, Some(rng(&g.ys)));
        }
        GateKind::Interval => {
            let _ = writeln!(out, "<gating:RectangleGate gating:id=\"{}\"{pa}>", escape(id));
            dim(out, &g.x_chan, Some(rng(&g.xs)));
        }
        GateKind::Polygon => {
            let _ = writeln!(out, "<gating:PolygonGate gating:id=\"{}\"{pa}>", escape(id));
            dim(out, &g.x_chan, None);
            dim(out, y()?, None);
            for (x, yv) in g.xs.iter().zip(&g.ys) {
                let _ = writeln!(out, "  <gating:vertex><gating:coordinate data-type:value=\"{x}\"/><gating:coordinate data-type:value=\"{yv}\"/></gating:vertex>");
            }
            out.push_str("</gating:PolygonGate>\n");
            return Ok(());
        }
        GateKind::Ellipse => {
            let _ = writeln!(out, "<gating:EllipsoidGate gating:id=\"{}\"{pa}>", escape(id));
            dim(out, &g.x_chan, None);
            dim(out, y()?, None);
            let (x0, x1) = rng(&g.xs);
            let (y0, y1) = rng(&g.ys);
            let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0);
            // Sigma = R diag(rx^2, ry^2) R^T with R the CCW rotation by `angle`; Mahalanobis distance <= 1
            let (c, s) = (g.angle.cos(), g.angle.sin());
            let (a, b, d) = (c * c * rx * rx + s * s * ry * ry, c * s * (rx * rx - ry * ry), s * s * rx * rx + c * c * ry * ry);
            let _ = writeln!(out, "  <gating:mean><gating:coordinate data-type:value=\"{cx}\"/><gating:coordinate data-type:value=\"{cy}\"/></gating:mean>");
            let _ = writeln!(out, "  <gating:covarianceMatrix>");
            let _ = writeln!(out, "    <gating:row><gating:entry data-type:value=\"{a}\"/><gating:entry data-type:value=\"{b}\"/></gating:row>");
            let _ = writeln!(out, "    <gating:row><gating:entry data-type:value=\"{b}\"/><gating:entry data-type:value=\"{d}\"/></gating:row>");
            let _ = writeln!(out, "  </gating:covarianceMatrix>");
            let _ = writeln!(out, "  <gating:distanceSquare data-type:value=\"1\"/>");
            out.push_str("</gating:EllipsoidGate>\n");
            return Ok(());
        }
        GateKind::Quadrant(_) => return Err("quadrant gates have no exact GatingML form here".into()),
        GateKind::Indexed(_) => return Err("index-set populations are not geometric gates".into()),
    }
    out.push_str(if matches!(g.kind, GateKind::Rectangle | GateKind::Interval) { "</gating:RectangleGate>\n" } else { unreachable!() });
    Ok(())
}

/// One `gateReference`, optionally complemented. GatingML allows only these as operands.
fn operand(e: &BoolExpr, ids: &HashMap<NodeId, String>) -> Result<String, String> {
    let (n, complement) = match e {
        BoolExpr::Ref(n) => (n, false),
        BoolExpr::Not(x) => match &**x {
            BoolExpr::Ref(n) => (n, true),
            _ => return Err("GatingML Boolean operands must be populations or their complements, not nested expressions".into()),
        },
        _ => return Err("GatingML Boolean operands must be populations or their complements, not nested expressions".into()),
    };
    let id = ids.get(n).ok_or("refers to a population that was not exported")?;
    let flag = if complement { " gating:use-as-complement=\"true\"" } else { "" };
    Ok(format!("  <gating:gateReference gating:ref=\"{}\"{flag}/>\n", escape(id)))
}

fn bool_xml(out: &mut String, e: &BoolExpr, ids: &HashMap<NodeId, String>) -> Result<(), String> {
    match e {
        BoolExpr::And(v) | BoolExpr::Or(v) => {
            let tag = if matches!(e, BoolExpr::And(_)) { "and" } else { "or" };
            let ops: Vec<String> = v.iter().map(|x| operand(x, ids)).collect::<Result<_, _>>()?;
            let _ = writeln!(out, " <gating:{tag}>\n{}</gating:{tag}>", ops.concat());
        }
        BoolExpr::Not(x) => match &**x {
            BoolExpr::Ref(_) => {
                let _ = writeln!(out, " <gating:not>\n{}</gating:not>", operand(x, ids)?);
            }
            _ => return Err("GatingML cannot express a negated compound expression".into()),
        },
        BoolExpr::Ref(_) => return Err("a Boolean gate must be an And, Or or Not".into()),
    }
    Ok(())
}

pub fn to_gatingml(tree: &GateTree, meta: &HashMap<NodeId, NodeMeta>) -> Export {
    let order = tree.self_and_descendants(GateTree::ROOT).expect("root exists");
    // file ids: the population name when it is unique in the whole tree, else name + node number
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for &n in order.iter().skip(1) {
        *seen.entry(tree.name(n).unwrap()).or_default() += 1;
    }
    let file_id = |n: NodeId| {
        let name = tree.name(n).unwrap();
        if seen[name] == 1 {
            name.to_string()
        } else {
            format!("{name} [{n}]")
        }
    };

    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gating:Gating-ML xmlns:gating=\"http://www.isac-net.org/std/Gating-ML/v2.0/gating\" xmlns:data-type=\"http://www.isac-net.org/std/Gating-ML/v2.0/datatypes\">\n",
    );
    let (mut exported, mut warnings): (Vec<(String, Vec<String>)>, Vec<String>) = (Vec::new(), Vec::new());
    let mut ids: HashMap<NodeId, String> = HashMap::new();
    let mut skipped: HashSet<NodeId> = HashSet::new();
    let mut pending_bool: Vec<NodeId> = Vec::new();

    for &n in order.iter().skip(1) {
        let path = tree.path(n).unwrap();
        let label = path.join(" > ");
        let parent = tree.parent(n).unwrap().unwrap();
        if skipped.contains(&parent) {
            skipped.insert(n);
            warnings.push(format!("'{label}': skipped because its parent was not exported"));
            continue;
        }
        match tree.kind(n).unwrap() {
            NodeKind::Root => unreachable!(),
            NodeKind::Boolean(_) => pending_bool.push(n),
            NodeKind::Gate(g) => {
                let m = meta.get(&n);
                let nonlinear = m.map_or(false, |m| !is_linear(&m.view_x_scale) || !is_linear(&m.view_y_scale));
                let problem = if !g.x_scale.is_identity() || !g.y_scale.is_identity() {
                    Some("it was drawn on a scaled axis; its coordinates are in display space, which GatingML would need the transform to express".to_string())
                } else if nonlinear {
                    Some("it is drawn on a transformed (logicle/arcsinh/log) axis; its coordinates are display-space values".to_string())
                } else if g.invert {
                    Some("inverted gates have no GatingML form".to_string())
                } else {
                    None
                };
                if let Some(p) = problem {
                    warnings.push(format!("'{label}': not exported — {p}"));
                    skipped.insert(n);
                    continue;
                }
                let id = file_id(n);
                let pid = if parent == GateTree::ROOT { None } else { ids.get(&parent).cloned() };
                let mut block = String::new();
                match gate_xml(&mut block, &id, pid.as_deref(), g) {
                    Ok(()) => {
                        if matches!(g.kind, GateKind::Rectangle | GateKind::Interval) && g.edges == EdgeRule::Inclusive {
                            warnings.push(format!("'{label}': inclusive upper edge — GatingML readers treat rectangle edges as half-open, so events exactly on the maximum may differ"));
                        }
                        out.push_str(&block);
                        ids.insert(n, id.clone());
                        exported.push((id, path));
                    }
                    Err(e) => {
                        warnings.push(format!("'{label}': not exported — {e}"));
                        skipped.insert(n);
                    }
                }
            }
        }
    }
    for n in pending_bool {
        let label = tree.path(n).unwrap().join(" > ");
        let NodeKind::Boolean(expr) = tree.kind(n).unwrap() else { unreachable!() };
        if tree.parent(n).unwrap() != Some(GateTree::ROOT) {
            warnings.push(format!("'{label}': not exported — GatingML Boolean gates cannot sit under a parent population"));
            continue;
        }
        let id = file_id(n);
        let mut block = format!("<gating:BooleanGate gating:id=\"{}\">\n", escape(&id));
        match bool_xml(&mut block, expr, &ids) {
            Ok(()) => {
                // a top-level Not/Ref is not a valid BooleanGate body in GatingML (needs and/or/not as the operator)
                block.push_str("</gating:BooleanGate>\n");
                out.push_str(&block);
                ids.insert(n, id.clone());
                exported.push((id, tree.path(n).unwrap()));
            }
            Err(e) => warnings.push(format!("'{label}': not exported — {e}")),
        }
    }
    out.push_str("</gating:Gating-ML>\n");
    Export { xml: out, exported, warnings }
}
