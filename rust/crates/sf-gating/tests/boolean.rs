//! Boolean mask gates, combination gates, apply-to-all idempotence and failure isolation,
//! all on a 10x10 grid whose answers can be counted by hand: event i has x = i % 10, y = i / 10.

use sf_gating::{BoolExpr, EdgeRule, EventView, Gate, GateTree, TreeError};

fn chans() -> Vec<String> {
    ["X", "Y"].iter().map(|s| s.to_string()).collect()
}

fn grid() -> Vec<f32> {
    (0..100).flat_map(|i| [(i % 10) as f32, (i / 10) as f32]).collect()
}

fn rect(x0: f64, x1: f64, y0: f64, y1: f64) -> Gate {
    Gate::rectangle("X", "Y", x0, x1, y0, y1).unwrap().with_edges(EdgeRule::Inclusive)
}

/// root -> A (x 0..4: 50 events), B (y 0..4: 50 events)
fn base() -> (GateTree, usize, usize) {
    let mut t = GateTree::new("All Events");
    let a = t.add_gate(GateTree::ROOT, "A", rect(0.0, 4.0, 0.0, 9.0)).unwrap();
    let b = t.add_gate(GateTree::ROOT, "B", rect(0.0, 9.0, 0.0, 4.0)).unwrap();
    (t, a, b)
}

#[test]
fn and_or_not_have_the_counts_set_theory_demands() {
    let (mut t, a, b) = base();
    let and = t.add_and(GateTree::ROOT, "A and B", &[a, b]).unwrap();
    let or = t.add_or(GateTree::ROOT, "A or B", &[a, b]).unwrap();
    let not = t.add_not(GateTree::ROOT, "not A", a).unwrap();
    let (data, c) = (grid(), chans());
    let ev = t.evaluate(&EventView::new(&data, 2, &c).unwrap()).unwrap();
    // |A|=|B|=50, |A and B| = 5x5 = 25, |A or B| = 50+50-25 = 75, |not A| = 50
    assert_eq!((ev.counts[and], ev.counts[or], ev.counts[not]), (25, 75, 50));
    // inclusion-exclusion and complement, straight from the masks
    assert_eq!(ev.counts[or], ev.counts[a] + ev.counts[b] - ev.counts[and]);
    assert_eq!(ev.counts[not] + ev.counts[a], 100);
    assert!(ev.warnings.is_empty());
}

#[test]
fn de_morgan_holds_on_real_populations() {
    let (mut t, a, b) = base();
    let n_and = t.add_boolean(GateTree::ROOT, "not(A and B)", BoolExpr::Not(Box::new(BoolExpr::And(vec![BoolExpr::Ref(a), BoolExpr::Ref(b)])))).unwrap();
    let or_nots = t
        .add_boolean(
            GateTree::ROOT,
            "notA or notB",
            BoolExpr::Or(vec![BoolExpr::Not(Box::new(BoolExpr::Ref(a))), BoolExpr::Not(Box::new(BoolExpr::Ref(b)))]),
        )
        .unwrap();
    let (data, c) = (grid(), chans());
    let ev = t.evaluate(&EventView::new(&data, 2, &c).unwrap()).unwrap();
    assert_eq!(ev.masks[n_and], ev.masks[or_nots]);
    assert_eq!(ev.counts[n_and], 75);
}

#[test]
fn a_boolean_gate_still_respects_its_parent_population() {
    let (mut t, a, b) = base();
    // P: x 0..1 (20 events). Under P, "A or B" = P and (A or B) = events with x<=1 and (x<=4 or y<=4) = 20.
    let p = t.add_gate(GateTree::ROOT, "P", rect(0.0, 1.0, 0.0, 9.0)).unwrap();
    let under = t.add_or(p, "A or B under P", &[a, b]).unwrap();
    // and under P with B: x<=1 and y<=4 = 2 x 5 = 10
    let under_b = t.add_and(p, "B under P", &[b]).unwrap();
    let (data, c) = (grid(), chans());
    let ev = t.evaluate(&EventView::new(&data, 2, &c).unwrap()).unwrap();
    assert_eq!((ev.counts[p], ev.counts[under], ev.counts[under_b]), (20, 20, 10));
}

#[test]
fn combination_gates_exactly_partition_the_parent() {
    let (mut t, a, b) = base();
    let combos = t.add_combination_gates(GateTree::ROOT, &[a, b], true).unwrap();
    assert_eq!(combos.len(), 4);
    let names: Vec<_> = combos.iter().map(|&c| t.name(c).unwrap().to_string()).collect();
    assert_eq!(names, vec!["A & B", "A & !B", "!A & B", "!A & !B"]);
    let (data, c) = (grid(), chans());
    let ev = t.evaluate(&EventView::new(&data, 2, &c).unwrap()).unwrap();
    let counts: Vec<usize> = combos.iter().map(|&c| ev.counts[c]).collect();
    assert_eq!(counts, vec![25, 25, 25, 25]);
    assert_eq!(counts.iter().sum::<usize>(), 100, "patterns must cover every event exactly once");
    for i in 0..4 {
        for j in i + 1..4 {
            let overlap = ev.masks[combos[i]].as_ref().unwrap().and(ev.masks[combos[j]].as_ref().unwrap()).count();
            assert_eq!(overlap, 0, "{} and {} overlap", names[i], names[j]);
        }
    }
    // without the all-negative pattern there are 2^n - 1 gates
    let (mut t2, a2, b2) = base();
    assert_eq!(t2.add_combination_gates(GateTree::ROOT, &[a2, b2], false).unwrap().len(), 3);
    assert!(t2.add_combination_gates(GateTree::ROOT, &[], true).is_err());
}

#[test]
fn replacing_a_gate_is_idempotent_and_duplicates_are_refused() {
    let (mut t, _, _) = base();
    for _ in 0..3 {
        t.replace_gate(GateTree::ROOT, "A", rect(0.0, 2.0, 0.0, 9.0)).unwrap();
    }
    let kids = t.children(GateTree::ROOT).unwrap().iter().filter(|&&c| t.name(c).unwrap() == "A").count();
    assert_eq!(kids, 1, "applying the same gate repeatedly must not stack duplicates (BUG-15)");
    assert_eq!(
        t.add_gate(GateTree::ROOT, "A", rect(0.0, 1.0, 0.0, 1.0)).unwrap_err(),
        TreeError::DuplicateName { parent: "All Events".into(), name: "A".into() }
    );
    let (data, c) = (grid(), chans());
    let ev = t.evaluate(&EventView::new(&data, 2, &c).unwrap()).unwrap();
    let a = t.find(&["A"]).unwrap();
    assert_eq!(ev.counts[a], 30, "the replacement (x 0..2) is the live gate");
}

#[test]
fn removing_a_population_orphans_booleans_with_a_warning_instead_of_a_wrong_number() {
    let (mut t, a, b) = base();
    let and = t.add_and(GateTree::ROOT, "A and B", &[a, b]).unwrap();
    assert_eq!(t.remove(a).unwrap(), 1);
    assert!(!t.is_alive(a) && t.is_alive(b));
    assert!(t.name(a).is_err());
    let (data, c) = (grid(), chans());
    let ev = t.evaluate(&EventView::new(&data, 2, &c).unwrap()).unwrap();
    assert_eq!(ev.counts[and], 0);
    assert_eq!(ev.counts[b], 50, "an unrelated sibling is untouched");
    assert_eq!(ev.warnings.len(), 1);
    assert!(ev.warnings[0].message.contains("deleted"), "{:?}", ev.warnings);
    assert!(t.remove(GateTree::ROOT).is_err(), "the root cannot be removed");
    // ids are never reused, so a stale id cannot silently point at a new population
    let c2 = t.add_gate(GateTree::ROOT, "C", rect(0.0, 1.0, 0.0, 1.0)).unwrap();
    assert!(c2 > and);
}

#[test]
fn one_bad_gate_does_not_abort_its_siblings() {
    let (mut t, _, b) = base();
    let bad = t.add_gate(GateTree::ROOT, "needs CD3", Gate::rectangle("X", "CD3", 0.0, 1.0, 0.0, 1.0).unwrap()).unwrap();
    let kid = t.add_gate(bad, "child of bad", rect(0.0, 9.0, 0.0, 9.0)).unwrap();
    let (data, c) = (grid(), chans());
    let ev = t.evaluate(&EventView::new(&data, 2, &c).unwrap()).unwrap();
    assert_eq!(ev.counts[bad], 0);
    assert_eq!(ev.counts[kid], 0, "children of an unevaluable population are empty too");
    assert_eq!(ev.counts[b], 50, "BUG-01: the other panels still render");
    assert_eq!(ev.warnings.len(), 1);
    assert_eq!(ev.warnings[0].name, "needs CD3");
    assert!(ev.warnings[0].message.contains("CD3"));
    assert!(ev.parent_pct[kid].is_nan(), "% of an empty parent is undefined, not 0 or 100");
}

#[test]
fn expressions_are_validated_when_they_are_built() {
    let (mut t, a, _) = base();
    assert!(matches!(t.add_boolean(GateTree::ROOT, "x", BoolExpr::And(vec![])), Err(TreeError::BadExpression(_))));
    assert!(matches!(t.add_boolean(GateTree::ROOT, "x", BoolExpr::Ref(999)), Err(TreeError::NoSuchNode(999))));
    let mut deep = BoolExpr::Ref(a);
    for _ in 0..70 {
        deep = BoolExpr::Not(Box::new(deep));
    }
    let err = t.add_boolean(GateTree::ROOT, "deep", deep).unwrap_err();
    assert!(err.to_string().contains("deep"), "{err}");
    assert!(t.add_gate(GateTree::ROOT, "  ", rect(0.0, 1.0, 0.0, 1.0)).is_err());
}
