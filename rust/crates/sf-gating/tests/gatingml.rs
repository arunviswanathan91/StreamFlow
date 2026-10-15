//! What the GatingML exporter will and will not write, and how it nests.

use sf_core::xml;
use sf_gating::gatingml::to_gatingml;
use sf_gating::{BoolExpr, EdgeRule, Gate, GateTree, NodeMeta, Quad};
use std::collections::HashMap;

fn rect(x0: f64, x1: f64) -> Gate {
    Gate::rectangle("X", "Y", x0, x1, 0.0, 1.0).unwrap()
}

#[test]
fn nested_gates_use_parent_id_and_duplicate_names_get_unique_ids() {
    let mut t = GateTree::new("All Events");
    let a = t.add_gate(GateTree::ROOT, "A", rect(0.0, 1.0)).unwrap();
    let b = t.add_gate(GateTree::ROOT, "B", rect(0.0, 1.0)).unwrap();
    t.add_gate(a, "Kid", rect(0.0, 1.0)).unwrap();
    t.add_gate(b, "Kid", rect(0.0, 1.0)).unwrap();
    let ex = to_gatingml(&t, &HashMap::new());
    assert!(ex.warnings.is_empty(), "{:?}", ex.warnings);
    let ids: Vec<&str> = ex.exported.iter().map(|(i, _)| i.as_str()).collect();
    assert_eq!(ids.len(), 4);
    assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), 4, "ids must be unique: {ids:?}");
    let doc = xml::parse(&ex.xml).unwrap();
    let kids: Vec<_> = doc.children.iter().filter(|g| g.attr("id").map_or(false, |i| i.starts_with("Kid"))).collect();
    assert_eq!(kids.len(), 2);
    let parents: Vec<_> = kids.iter().map(|k| k.attr("parent_id").unwrap()).collect();
    assert!(parents.contains(&"A") && parents.contains(&"B"));
}

#[test]
fn what_cannot_be_said_exactly_is_skipped_with_a_reason_and_takes_its_subtree_with_it() {
    let mut t = GateTree::new("All Events");
    let logi = t.add_gate(GateTree::ROOT, "OnLogicle", rect(0.0, 1.0)).unwrap();
    t.add_gate(logi, "Child", rect(0.0, 1.0)).unwrap();
    t.add_gate(GateTree::ROOT, "Inverted", rect(0.0, 1.0).inverted()).unwrap();
    t.add_gate(GateTree::ROOT, "Quad", Gate::quadrant("X", "Y", Quad::Q1, 0.5, 0.5).unwrap()).unwrap();
    t.add_gate(GateTree::ROOT, "Idx", Gate::indexed(vec![1, 2])).unwrap();
    let ok = t.add_gate(GateTree::ROOT, "Fine", rect(0.0, 1.0)).unwrap();
    t.add_not(ok, "NestedBool", ok).unwrap();
    let mut meta = HashMap::new();
    meta.insert(logi, NodeMeta { view_x_scale: Some("Logicle".into()), ..Default::default() });
    let ex = to_gatingml(&t, &meta);
    let w = ex.warnings.join("\n");
    for needle in ["OnLogicle", "transformed", "Child", "parent was not exported", "Inverted", "Quad", "Idx", "NestedBool", "parent population"] {
        assert!(w.contains(needle), "missing {needle:?} in:\n{w}");
    }
    assert_eq!(ex.exported.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(), vec!["Fine"]);
    // an explicit Linear scale is fine
    let mut meta2 = HashMap::new();
    meta2.insert(logi, NodeMeta { view_x_scale: Some("Linear".into()), ..Default::default() });
    assert!(to_gatingml(&t, &meta2).exported.iter().any(|(i, _)| i == "OnLogicle"));
}

#[test]
fn inclusive_gates_are_exported_but_flagged_and_booleans_use_complement_flags() {
    let mut t = GateTree::new("All Events");
    let a = t.add_gate(GateTree::ROOT, "A", rect(0.0, 1.0).with_edges(EdgeRule::Inclusive)).unwrap();
    let b = t.add_gate(GateTree::ROOT, "B", rect(0.0, 1.0)).unwrap();
    t.add_boolean(GateTree::ROOT, "AnotB", BoolExpr::And(vec![BoolExpr::Ref(a), BoolExpr::Not(Box::new(BoolExpr::Ref(b)))])).unwrap();
    t.add_boolean(GateTree::ROOT, "Compound", BoolExpr::Not(Box::new(BoolExpr::And(vec![BoolExpr::Ref(a), BoolExpr::Ref(b)])))).unwrap();
    let ex = to_gatingml(&t, &HashMap::new());
    let w = ex.warnings.join("\n");
    assert!(w.contains("inclusive upper edge") && w.contains("Compound"), "{w}");
    assert!(ex.xml.contains("gating:use-as-complement=\"true\""));
    assert!(ex.exported.iter().any(|(i, _)| i == "AnotB") && !ex.exported.iter().any(|(i, _)| i == "Compound"));
    xml::parse(&ex.xml).unwrap();
}

#[test]
fn channel_names_with_markup_characters_are_escaped() {
    let mut t = GateTree::new("All Events");
    t.add_gate(GateTree::ROOT, "a<&>\"", Gate::rectangle("CD3 <&> \"x\"", "Y", 0.0, 1.0, 0.0, 1.0).unwrap()).unwrap();
    let doc = xml::parse(&to_gatingml(&t, &HashMap::new()).xml).unwrap();
    assert_eq!(doc.children[0].attr("id"), Some("a<&>\""));
    assert_eq!(doc.children[0].children[0].child("fcs-dimension").unwrap().attr("name"), Some("CD3 <&> \"x\""));
}
