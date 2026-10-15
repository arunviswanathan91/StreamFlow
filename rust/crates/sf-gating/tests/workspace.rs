//! Workspace persistence: files written by the Java app load unchanged, our own files round-trip
//! exactly (gates, Boolean expressions, view state, counts), and damaged files degrade with warnings.

use sf_core::json::Json;
use sf_gating::workspace::{gates_from_json, gates_to_json, workspace_from_json, workspace_to_json};
use sf_gating::{BoolExpr, EdgeRule, EventView, Gate, GateKind, GateTree, NodeKind, NodeMeta};
use std::collections::{BTreeMap, HashMap};

/// A gates object exactly as `MainController.serializeGates` writes it (legacy keys, no `invert`).
const JAVA_SFW: &str = r#"{
 "S1.fcs": [
  {"id":"G1","name":"Lymphocytes","type":"polygon","x_channel":"FSC-A","y_channel":"SSC-A","angle":0.0,
   "xs":[10,90,90,10],"ys":[10,10,90,90],"view_x":"FSC-A","view_y":"SSC-A","view_x_scale":"Linear","view_y_scale":"Linear","count":55,"parent_pct":55.0},
  {"id":"G2","parent_id":"G1","name":"CD3+","type":"rectangle","x_channel":"CD3","y_channel":"CD4","angle":0.0,
   "xs":[2.0,8.0],"ys":[1.0,9.0],"lbl_dx":3.0,"lbl_dy":-4.0,"count":20},
  {"id":"G3","parent_id":"G2","name":"CD4+","type":"q1","x_channel":"CD3","y_channel":"CD4","angle":0.0,"xs":[5.0],"ys":[5.0]},
  {"id":"G4","parent_id":"G3","name":"CD25+","type":"ellipse","x_channel":"CD25","y_channel":"FOXP3","angle":0.5,"xs":[1,3],"ys":[1,3]},
  {"id":"G5","parent_id":"G4","name":"FoxP3+","type":"interval","x_channel":"FOXP3","xs":[2,3],"ys":[]},
  {"id":"G6","name":"UMAP pop","type":"embedding","x_channel":"UMAP1","y_channel":"UMAP2","angle":0,"xs":[],"ys":[],"plugin_indices":[0,5,7]}
 ]
}"#;

fn channels() -> Vec<String> {
    ["FSC-A", "SSC-A", "CD3", "CD4", "CD25", "FOXP3", "UMAP1", "UMAP2"].iter().map(|s| s.to_string()).collect()
}

fn events() -> Vec<f32> {
    // 100 deterministic rows x 8 columns; values wrap so every gate has some members
    (0..100).flat_map(|i| (0..8).map(move |c| (((i * 7 + c * 13) % 100) as f32) / 10.0 + if c < 2 { 5.0 } else { 0.0 })).collect()
}

#[test]
fn a_java_written_workspace_loads_with_its_five_level_chain_and_view_state() {
    let doc = Json::parse(JAVA_SFW).unwrap();
    let (ws, warnings) = workspace_from_json(&doc, "All Events").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let s = &ws["S1.fcs"];
    // golden invariant: the Lymphocytes -> CD3+ -> CD4+ -> CD25+ -> FoxP3+ chain is present at depth 5
    let deep = s.tree.find(&["Lymphocytes", "CD3+", "CD4+", "CD25+", "FoxP3+"]).expect("5-level population");
    assert_eq!(s.tree.depth(deep).unwrap(), 5);
    let lym = s.tree.find(&["Lymphocytes"]).unwrap();
    let m = &s.meta[&lym];
    assert_eq!((m.view_x.as_deref(), m.view_y_scale.as_deref(), m.count, m.parent_pct), (Some("FSC-A"), Some("Linear"), Some(55), Some(55.0)));
    let cd3 = s.tree.find(&["Lymphocytes", "CD3+"]).unwrap();
    assert_eq!((s.meta[&cd3].lbl_dx, s.meta[&cd3].count, s.meta[&cd3].parent_pct), (Some(3.0), Some(20), None));
    match s.tree.kind(s.tree.find(&["UMAP pop"]).unwrap()).unwrap() {
        NodeKind::Gate(Gate { kind: GateKind::Indexed(rows), .. }) => assert_eq!(rows, &[0u32, 5, 7][..]),
        k => panic!("{k:?}"),
    }
    let ell = s.tree.find(&["Lymphocytes", "CD3+", "CD4+", "CD25+"]).unwrap();
    match s.tree.kind(ell).unwrap() {
        NodeKind::Gate(g) => assert_eq!((g.angle, g.edges, g.invert), (0.5, EdgeRule::Inclusive, false)),
        k => panic!("{k:?}"),
    }
}

#[test]
fn save_then_load_gives_the_same_populations_and_the_same_event_counts() {
    let doc = Json::parse(JAVA_SFW).unwrap();
    let (ws, _) = workspace_from_json(&doc, "All Events").unwrap();
    let s = &ws["S1.fcs"];

    // add what Java cannot express: an inverted gate, a half-open gate and Boolean gates
    let mut tree = s.tree.clone();
    let mut meta = s.meta.clone();
    let a = tree.find(&["Lymphocytes", "CD3+"]).unwrap();
    let neg = tree
        .add_gate(GateTree::ROOT, "outside", Gate::rectangle("CD3", "CD4", 2.0, 8.0, 1.0, 9.0).unwrap().inverted().with_edges(EdgeRule::HalfOpen))
        .unwrap();
    let lym = tree.find(&["Lymphocytes"]).unwrap();
    let both = tree.add_and(GateTree::ROOT, "Lym and outside", &[lym, neg]).unwrap();
    let combo = tree.add_boolean(a, "nested boolean", BoolExpr::Not(Box::new(BoolExpr::Or(vec![BoolExpr::Ref(lym), BoolExpr::Ref(neg)])))).unwrap();
    meta.insert(both, NodeMeta { count: Some(3), ..Default::default() });

    let (data, c) = (events(), channels());
    let before = tree.evaluate(&EventView::new(&data, 8, &c).unwrap()).unwrap();

    let text = gates_to_json(&tree, &meta).write();
    let (back, warnings) = gates_from_json(&Json::parse(&text).unwrap(), "All Events").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.tree.len(), tree.len());

    let after = back.tree.evaluate(&EventView::new(&data, 8, &c).unwrap()).unwrap();
    for (path, id) in [(vec!["outside"], neg), (vec!["Lym and outside"], both), (vec!["Lymphocytes", "CD3+", "nested boolean"], combo)] {
        let nid = back.tree.find(&path).unwrap_or_else(|| panic!("{path:?} lost"));
        assert_eq!(after.counts[nid], before.counts[id], "{path:?}");
        assert_eq!(after.masks[nid], before.masks[id], "{path:?}");
    }
    let deep = back.tree.find(&["Lymphocytes", "CD3+", "CD4+", "CD25+", "FoxP3+"]).unwrap();
    let deep0 = tree.find(&["Lymphocytes", "CD3+", "CD4+", "CD25+", "FoxP3+"]).unwrap();
    assert_eq!(after.counts[deep], before.counts[deep0]);
    // inverted + half-open survived
    match back.tree.kind(back.tree.find(&["outside"]).unwrap()).unwrap() {
        NodeKind::Gate(g) => assert!(g.invert && g.edges == EdgeRule::HalfOpen),
        k => panic!("{k:?}"),
    }
    // saving what we loaded is a fixed point (stable ids and order, deterministic keys)
    assert_eq!(gates_to_json(&back.tree, &back.meta).write(), text);
    // the original type of an index population is kept for the Java app
    assert!(text.contains("\"type\":\"embedding\""));
}

#[test]
fn whole_workspace_skips_ungated_samples_and_reports_prefixed_warnings() {
    let mut samples = BTreeMap::new();
    samples.insert("empty.fcs".to_string(), sf_gating::SampleGates { tree: GateTree::new("All Events"), meta: HashMap::new() });
    let v = workspace_to_json(&samples);
    assert_eq!(v.as_obj().unwrap().len(), 0);

    let bad = Json::parse(r#"{"X.fcs":[{"id":"G1","name":"n","type":"hexagon"}]}"#).unwrap();
    let (ws, w) = workspace_from_json(&bad, "All Events").unwrap();
    assert_eq!(ws["X.fcs"].tree.len(), 1);
    assert!(w[0].starts_with("X.fcs: ") && w[0].contains("hexagon"), "{w:?}");
    assert!(workspace_from_json(&Json::parse("[1]").unwrap(), "r").is_err());
    assert!(workspace_from_json(&Json::Null, "r").unwrap().0.is_empty());
}

#[test]
fn damaged_records_are_repaired_or_dropped_loudly_never_silently() {
    let doc = Json::parse(
        r#"[
 {"id":"G1","name":"A","type":"rectangle","x_channel":"X","y_channel":"Y","xs":[0,1],"ys":[0,1]},
 {"id":"G2","name":"A","type":"rectangle","x_channel":"X","y_channel":"Y","xs":[0,2],"ys":[0,2]},
 {"id":"G3","parent_id":"G2","name":"child of dup","type":"interval","x_channel":"X","xs":[0,1]},
 {"id":"G4","parent_id":"GHOST","name":"orphan","type":"interval","x_channel":"X","xs":[0,1]},
 {"id":"G5","name":"sub","type":"subsample","x_channel":"X","sub_n":10,"sub_seed":1},
 {"id":"G6","name":"bad poly","type":"polygon","x_channel":"X","y_channel":"Y","xs":[0,1],"ys":[0,1]},
 {"id":"G1","name":"dup id","type":"interval","x_channel":"X","xs":[0,1]},
 {"id":"G7","name":"bool to nothing","type":"boolean","expr":{"op":"ref","id":"nope"}},
 {"id":"G8","name":"cyc1","parent_id":"G9","type":"interval","x_channel":"X","xs":[0,1]},
 {"id":"G9","name":"cyc2","parent_id":"G8","type":"interval","x_channel":"X","xs":[0,1]}
]"#,
    )
    .unwrap();
    let (s, w) = gates_from_json(&doc, "All Events").unwrap();
    let names: Vec<_> = s.tree.self_and_descendants(GateTree::ROOT).unwrap().iter().map(|&n| s.tree.name(n).unwrap().to_string()).collect();
    assert_eq!(names, vec!["All Events", "A", "orphan"], "{w:?}");
    let all = w.join("\n");
    for needle in ["child of dup", "orphan", "missing parent", "subsample", "bad poly", "duplicate id", "bool to nothing", "cyc1", "cyc2"] {
        assert!(all.contains(needle), "no warning mentioning {needle:?}:\n{all}");
    }
    // the first of two same-named siblings wins (BUG-15 auto-heal) — the surviving 'A' is the unit square
    match s.tree.kind(s.tree.find(&["A"]).unwrap()).unwrap() {
        NodeKind::Gate(g) => assert_eq!(g.xs, vec![0.0, 1.0]),
        k => panic!("{k:?}"),
    }
}

#[test]
fn a_ten_thousand_level_chain_saves_and_loads_without_recursion() {
    let mut t = GateTree::new("All Events");
    let mut cur = GateTree::ROOT;
    for i in 0..10_000 {
        cur = t.add_gate(cur, &format!("L{i}"), Gate::interval("X", 0.0, 1.0).unwrap()).unwrap();
    }
    let text = gates_to_json(&t, &HashMap::new()).write();
    let (back, w) = gates_from_json(&Json::parse(&text).unwrap(), "All Events").unwrap();
    assert!(w.is_empty());
    assert_eq!(back.tree.len(), 10_001);
    let last = back.tree.self_and_descendants(GateTree::ROOT).unwrap().pop().unwrap();
    assert_eq!(back.tree.depth(last).unwrap(), 10_000);
}

#[test]
fn a_boolean_may_reference_a_population_that_appears_later_in_the_file() {
    let doc = Json::parse(
        r#"[
 {"id":"G1","name":"both","type":"boolean","expr":{"op":"and","args":[{"op":"ref","id":"G2"},{"op":"not","arg":{"op":"ref","id":"G3"}}]}},
 {"id":"G2","name":"a","type":"interval","x_channel":"X","xs":[0,5]},
 {"id":"G3","name":"b","type":"interval","x_channel":"X","xs":[3,9]}
]"#,
    )
    .unwrap();
    let (s, w) = gates_from_json(&doc, "All Events").unwrap();
    assert!(w.is_empty(), "{w:?}");
    let ch = vec!["X".to_string()];
    let data: Vec<f32> = (0..10).map(|v| v as f32).collect();
    let ev = s.tree.evaluate(&EventView::new(&data, 1, &ch).unwrap()).unwrap();
    // a = 0..=5, b = 3..=9, a and not b = 0,1,2
    assert_eq!(ev.counts[s.tree.find(&["both"]).unwrap()], 3);
}
