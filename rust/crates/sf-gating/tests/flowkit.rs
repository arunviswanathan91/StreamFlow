//! Real-data parity: the SAME gates, built from the numbers in `reference_gating.txt`, evaluated on the
//! real fixture by the Rust engine, must give the event counts FlowKit's gating engine gave.
//! (Regenerate the reference with `rust/tools/gen_gating_reference.py`.)

use sf_fcs::Fcs;
use sf_gating::{BoolExpr, EdgeRule, EventView, Gate, GateKind, GateTree, NodeId};
use std::collections::HashMap;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/fcs/multi stain/AF700 alone.fcs");
const REFERENCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/reference_gating.txt");

struct Case {
    name: String,
    gate: Option<Gate>,
    /// FlowKit's count (GatingML semantics).
    flowkit: usize,
    /// For rect/interval only: the plain inclusive count recorded by NumPy.
    inclusive: Option<usize>,
    bool_def: Option<(String, Vec<(String, bool)>)>,
}

fn f(tok: &str) -> f64 {
    tok.parse().unwrap_or_else(|_| panic!("bad number {tok:?}"))
}

fn load_reference() -> (usize, Vec<Case>) {
    let text = std::fs::read_to_string(REFERENCE).expect("reference_gating.txt (run rust/tools/gen_gating_reference.py)");
    let (mut n_events, mut cases) = (0, Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let t: Vec<&str> = line.split_whitespace().collect();
        match t[0] {
            "events" => n_events = t[1].parse().unwrap(),
            "rect" => cases.push(Case {
                name: t[1].into(),
                gate: Some(Gate::rectangle(t[2], t[3], f(t[4]), f(t[5]), f(t[6]), f(t[7])).unwrap()),
                flowkit: t[8].parse().unwrap(),
                inclusive: Some(t[9].parse().unwrap()),
                bool_def: None,
            }),
            "interval" => cases.push(Case {
                name: t[1].into(),
                gate: Some(Gate::interval(t[2], f(t[3]), f(t[4])).unwrap()),
                flowkit: t[5].parse().unwrap(),
                inclusive: Some(t[6].parse().unwrap()),
                bool_def: None,
            }),
            "poly" => {
                let n: usize = t[4].parse().unwrap();
                let verts: Vec<(f64, f64)> = (0..n).map(|k| (f(t[5 + 2 * k]), f(t[6 + 2 * k]))).collect();
                cases.push(Case {
                    name: t[1].into(),
                    gate: Some(Gate::polygon(t[2], t[3], &verts).unwrap()),
                    flowkit: t[5 + 2 * n].parse().unwrap(),
                    inclusive: None,
                    bool_def: None,
                });
            }
            "ellipse" => cases.push(Case {
                name: t[1].into(),
                gate: Some(Gate::ellipse(t[2], t[3], f(t[4]), f(t[5]), f(t[6]), f(t[7]), f(t[8])).unwrap()),
                flowkit: t[9].parse().unwrap(),
                inclusive: None,
                bool_def: None,
            }),
            "bool" => cases.push(Case {
                name: t[1].into(),
                gate: None,
                flowkit: t[4].parse().unwrap(),
                inclusive: None,
                bool_def: Some((t[2].into(), t[3].split(',').map(|r| (r.trim_start_matches('!').to_string(), r.starts_with('!'))).collect())),
            }),
            other => panic!("unknown reference line {other}"),
        }
    }
    (n_events, cases)
}

/// Build the tree for one edge rule, evaluate on the fixture, return `name -> count`.
fn counts_with(edges: EdgeRule) -> (usize, HashMap<String, usize>, HashMap<String, usize>) {
    let (n, got, want, _) = build(edges);
    (n, got, want)
}

fn build(edges: EdgeRule) -> (usize, HashMap<String, usize>, HashMap<String, usize>, GateTree) {
    let fcs = Fcs::read(FIXTURE).expect("fixture");
    let channels: Vec<String> = fcs.params.iter().map(|p| p.name.clone()).collect();
    let view = EventView::new(&fcs.events, fcs.n_params(), &channels).unwrap();
    let (n_events, cases) = load_reference();
    assert_eq!(fcs.n_events, n_events, "fixture and reference disagree about the event count");

    let mut t = GateTree::new("root");
    let mut ids: HashMap<String, NodeId> = HashMap::new();
    for c in cases.iter().filter(|c| c.gate.is_some()) {
        let mut g = c.gate.clone().unwrap();
        if matches!(g.kind, GateKind::Rectangle | GateKind::Interval) {
            g = g.with_edges(edges);
        }
        ids.insert(c.name.clone(), t.add_gate(GateTree::ROOT, &c.name, g).unwrap());
    }
    for c in cases.iter().filter(|c| c.bool_def.is_some()) {
        let (op, refs) = c.bool_def.clone().unwrap();
        let operands: Vec<BoolExpr> = refs
            .iter()
            .map(|(n, comp)| if *comp { BoolExpr::Not(Box::new(BoolExpr::Ref(ids[n]))) } else { BoolExpr::Ref(ids[n]) })
            .collect();
        let expr = match op.as_str() {
            "and" => BoolExpr::And(operands),
            "or" => BoolExpr::Or(operands),
            "not" => BoolExpr::Not(Box::new(operands.into_iter().next().unwrap())),
            o => panic!("op {o}"),
        };
        ids.insert(c.name.clone(), t.add_boolean(GateTree::ROOT, &c.name, expr).unwrap());
    }
    let ev = t.evaluate(&view).unwrap();
    assert!(ev.warnings.is_empty(), "{:?}", ev.warnings);
    let got: HashMap<String, usize> = ids.iter().map(|(k, &id)| (k.clone(), ev.counts[id])).collect();
    let want: HashMap<String, usize> = cases.iter().map(|c| (c.name.clone(), c.flowkit)).collect();
    (n_events, got, want, t)
}

#[test]
fn every_gate_and_boolean_matches_flowkit_exactly_with_gatingml_half_open_edges() {
    let (_, got, want) = counts_with(EdgeRule::HalfOpen);
    let mut bad = Vec::new();
    for (name, w) in &want {
        if got[name] != *w {
            bad.push(format!("{name}: Rust {} vs FlowKit {w} (diff {})", got[name], got[name] as i64 - *w as i64));
        }
    }
    bad.sort();
    assert!(bad.is_empty(), "count mismatches against FlowKit:\n{}", bad.join("\n"));
    assert_eq!(got.len(), 14, "all reference gates must be exercised");
}

#[test]
fn the_default_inclusive_rule_reproduces_the_java_style_counts_and_really_differs_on_quantised_data() {
    let (_, got, _) = counts_with(EdgeRule::Inclusive);
    let (_, cases) = load_reference();
    let mut max_gap = 0i64;
    for c in cases.iter().filter(|c| c.inclusive.is_some()) {
        assert_eq!(got[&c.name], c.inclusive.unwrap(), "{}: inclusive count must equal NumPy's `>= min & <= max`", c.name);
        max_gap = max_gap.max(c.inclusive.unwrap() as i64 - c.flowkit as i64);
    }
    // the fluorescence pair is quantised (424 distinct FITC values) so the two rules disagree widely
    assert!(max_gap > 1000, "expected the edge rules to differ materially on this fixture, gap {max_gap}");
}

/// Writes the Rust-built gates as GatingML plus the counts Rust computed, for `rust/tools/check_gatingml.py`
/// to reload in FlowKit. Also checks the file is well-formed and lists every gate it claims to export.
#[test]
fn gatingml_export_is_well_formed_and_complete_for_the_reference_gates() {
    let (_, got, _, tree) = build(EdgeRule::HalfOpen);
    let ex = sf_gating::gatingml::to_gatingml(&tree, &HashMap::new());
    assert!(ex.warnings.is_empty(), "{:?}", ex.warnings);
    assert_eq!(ex.exported.len(), 14);
    let doc = sf_core::xml::parse(&ex.xml).expect("well-formed XML");
    assert_eq!(doc.name, "Gating-ML");
    assert_eq!(doc.children.len(), 14);
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::write(dir.join("export.xml"), &ex.xml).unwrap();
    let mut counts: Vec<String> = got.iter().map(|(k, v)| format!("{k} {v}")).collect();
    counts.sort();
    std::fs::write(dir.join("export_counts.txt"), counts.join("\n") + "\n").unwrap();
    eprintln!("GATINGML_DIR={}", dir.display());
}

/// Same gates nested three deep (rect -> ellipse -> polygon), exported with `parent_id`, for FlowKit to reload.
#[test]
fn nested_gatingml_export_for_flowkit() {
    let fcs = Fcs::read(FIXTURE).expect("fixture");
    let channels: Vec<String> = fcs.params.iter().map(|p| p.name.clone()).collect();
    let view = EventView::new(&fcs.events, fcs.n_params(), &channels).unwrap();
    let (_, cases) = load_reference();
    let pick = |n: &str| cases.iter().find(|c| c.name == n).unwrap().gate.clone().unwrap();
    let mut t = GateTree::new("root");
    let a = t.add_gate(GateTree::ROOT, "outer", pick("rect_FSC").with_edges(EdgeRule::HalfOpen)).unwrap();
    let b = t.add_gate(a, "middle", pick("e1_FSC")).unwrap();
    let c = t.add_gate(b, "inner", pick("poly_FSC")).unwrap();
    let ev = t.evaluate(&view).unwrap();
    assert!(ev.counts[c] > 0 && ev.counts[c] < ev.counts[b] && ev.counts[b] < ev.counts[a]);
    let ex = sf_gating::gatingml::to_gatingml(&t, &HashMap::new());
    assert!(ex.warnings.is_empty(), "{:?}", ex.warnings);
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::write(dir.join("nested.xml"), &ex.xml).unwrap();
    std::fs::write(dir.join("nested_counts.txt"), format!("outer {}\nmiddle {}\ninner {}\n", ev.counts[a], ev.counts[b], ev.counts[c])).unwrap();
}
