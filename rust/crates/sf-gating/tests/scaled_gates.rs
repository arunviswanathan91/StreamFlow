//! Gates drawn on a scaled axis are tested in that axis's display space.

use sf_core::json::Json;
use sf_core::rng::Rng;
use sf_core::scale::ScaleSpec;
use sf_gating::gatingml::to_gatingml;
use sf_gating::workspace::{gates_from_json, gates_to_json};
use sf_gating::{EdgeRule, EventView, Gate, GateTree};
use std::collections::HashMap;

fn channels() -> Vec<String> {
    ["X", "Y"].iter().map(|s| s.to_string()).collect()
}

/// 2,000 events spread over several decades, including negatives (as compensated data has).
fn events() -> Vec<f32> {
    let mut rng = Rng::new(21);
    (0..2000).flat_map(|_| [((rng.next_f64() - 0.2) * 20_000.0) as f32, ((rng.next_f64() - 0.1) * 5_000.0) as f32]).collect()
}

fn arcsinh(c: f64) -> ScaleSpec {
    ScaleSpec::Arcsinh { cofactor: c }
}

#[test]
fn a_rectangle_on_an_arcsinh_axis_selects_exactly_the_raw_range_it_maps_to() {
    let (data, ch) = (events(), channels());
    let t = arcsinh(150.0).build().unwrap();
    let g = Gate::rectangle("X", "Y", t.forward(100.0), t.forward(5000.0), -1e9, 1e9).unwrap().with_scales(arcsinh(150.0), ScaleSpec::Identity);
    let mut tree = GateTree::new("r");
    let id = tree.add_gate(GateTree::ROOT, "G", g).unwrap();
    let ev = tree.evaluate(&EventView::new(&data, 2, &ch).unwrap()).unwrap();
    let want = data.chunks(2).filter(|r| (100.0..5000.0).contains(&(r[0] as f64))).count();
    // arcsinh is monotone, so only events within float rounding of an edge could differ
    assert!((ev.counts[id] as i64 - want as i64).abs() <= 2, "{} vs {want}", ev.counts[id]);
    assert!(want > 100);
}

#[test]
fn a_polygon_is_tested_in_display_space_not_raw_space() {
    let (data, ch) = (events(), channels());
    let sc = ScaleSpec::Logicle { t: 262144.0, w: 0.5, m: 4.5, a: 0.0 };
    let t = sc.build().unwrap();
    // a triangle drawn on the screen (display-space vertices)
    let tri = [(t.forward(-2000.0), 200.0), (t.forward(15000.0), 200.0), (t.forward(500.0), 4000.0)];

    // (a) the gate carrying its scale
    let screen = Gate::polygon("X", "Y", &tri).unwrap().with_scales(sc.clone(), ScaleSpec::Identity);
    // (b) reference: pre-transform every event, test with an identity-scale polygon
    let pre: Vec<f32> = data.chunks(2).flat_map(|r| [t.forward(r[0] as f64) as f32, r[1]]).collect();
    let plain = Gate::polygon("X", "Y", &tri).unwrap();
    let mut t1 = GateTree::new("r");
    let a = t1.add_gate(GateTree::ROOT, "S", screen).unwrap();
    let e1 = t1.evaluate(&EventView::new(&data, 2, &ch).unwrap()).unwrap();
    let mut t2 = GateTree::new("r");
    let b = t2.add_gate(GateTree::ROOT, "S", plain).unwrap();
    let e2 = t2.evaluate(&EventView::new(&pre, 2, &ch).unwrap()).unwrap();
    assert_eq!(e1.masks[a], e2.masks[b], "scaled gate must equal 'transform, then test'");

    // (c) what the Java app does: convert the vertices to RAW units and test the polygon there. On a curved
    // axis the edges bend, so the selected cells differ from what was drawn.
    let raw_tri: Vec<(f64, f64)> = tri.iter().map(|&(x, y)| (t.inverse(x), y)).collect();
    let raw_space = Gate::polygon("X", "Y", &raw_tri).unwrap();
    let mut t3 = GateTree::new("r");
    let c = t3.add_gate(GateTree::ROOT, "S", raw_space).unwrap();
    let e3 = t3.evaluate(&EventView::new(&data, 2, &ch).unwrap()).unwrap();
    let differ = e1.masks[a].as_ref().unwrap().and(&e3.masks[c].as_ref().unwrap().not()).count() + e3.masks[c].as_ref().unwrap().and(&e1.masks[a].as_ref().unwrap().not()).count();
    eprintln!("drawn {} events, raw-space polygon {} events, {differ} events differ", e1.counts[a], e3.counts[c]);
    assert!(differ > 20, "the two interpretations should visibly disagree on a logicle axis (got {differ})");
}

#[test]
fn an_interval_leaves_the_y_column_alone_and_a_bad_scale_is_a_named_warning() {
    let (data, ch) = (events(), channels());
    let g = Gate::interval("X", 0.0, 1.0).unwrap().with_scales(arcsinh(150.0), arcsinh(5.0)).with_edges(EdgeRule::Inclusive);
    let mut tree = GateTree::new("r");
    let i = tree.add_gate(GateTree::ROOT, "I", g).unwrap();
    let bad = tree.add_gate(GateTree::ROOT, "Bad", Gate::interval("X", 0.0, 1.0).unwrap().with_scales(ScaleSpec::Arcsinh { cofactor: -1.0 }, ScaleSpec::Identity)).unwrap();
    let ev = tree.evaluate(&EventView::new(&data, 2, &ch).unwrap()).unwrap();
    assert!(ev.counts[i] > 0);
    assert_eq!(ev.counts[bad], 0);
    assert_eq!(ev.warnings.len(), 1);
    assert!(ev.warnings[0].message.contains("cofactor"), "{:?}", ev.warnings);
}

#[test]
fn scales_survive_save_and_load_and_identity_is_not_written() {
    let mut t = GateTree::new("r");
    let sc = ScaleSpec::Logicle { t: 262144.0, w: 0.5, m: 4.5, a: 0.0 };
    t.add_gate(GateTree::ROOT, "scaled", Gate::rectangle("X", "Y", 0.1, 0.9, 0.0, 1.0).unwrap().with_scales(sc.clone(), arcsinh(150.0))).unwrap();
    t.add_gate(GateTree::ROOT, "plain", Gate::rectangle("X", "Y", 0.0, 1.0, 0.0, 1.0).unwrap()).unwrap();
    let text = gates_to_json(&t, &HashMap::new()).write();
    assert_eq!(text.matches("x_scale").count(), 1, "only the scaled gate writes a scale: {text}");
    let (back, w) = gates_from_json(&Json::parse(&text).unwrap(), "r").unwrap();
    assert!(w.is_empty(), "{w:?}");
    let get = |name: &str| match back.tree.kind(back.tree.find(&[name]).unwrap()).unwrap() {
        sf_gating::NodeKind::Gate(g) => g.clone(),
        k => panic!("{k:?}"),
    };
    assert_eq!((get("scaled").x_scale, get("scaled").y_scale), (sc, arcsinh(150.0)));
    assert!(get("plain").x_scale.is_identity());
    // a corrupt scale string drops that record with a reason instead of guessing
    let bad = text.replace("arcsinh:150", "arcsinh:abc");
    let (b2, w2) = gates_from_json(&Json::parse(&bad).unwrap(), "r").unwrap();
    assert!(b2.tree.find(&["scaled"]).is_none() && w2.iter().any(|m| m.contains("scaled") && m.contains("bad number")), "{w2:?}");
}

#[test]
fn gatingml_export_refuses_scaled_gates_by_name() {
    let mut t = GateTree::new("r");
    t.add_gate(GateTree::ROOT, "S", Gate::rectangle("X", "Y", 0.0, 1.0, 0.0, 1.0).unwrap().with_scales(arcsinh(150.0), ScaleSpec::Identity)).unwrap();
    t.add_gate(GateTree::ROOT, "P", Gate::rectangle("X", "Y", 0.0, 1.0, 0.0, 1.0).unwrap()).unwrap();
    let ex = to_gatingml(&t, &HashMap::new());
    assert_eq!(ex.exported.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(), vec!["P"]);
    assert!(ex.warnings.iter().any(|w| w.contains("'S'") && w.contains("scaled axis")), "{:?}", ex.warnings);
}
