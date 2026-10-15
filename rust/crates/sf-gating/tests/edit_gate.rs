//! Editing a gate's geometry in place keeps the gate's name, its place in the tree and everything gated below it.

use sf_gating::{EventView, Gate, GateTree, NodeKind};

#[test]
fn set_gate_changes_membership_but_keeps_children_and_rejects_a_different_gate() {
    let channels = vec!["X".to_string(), "Y".to_string()];
    let data: Vec<f32> = (0..10).flat_map(|i| (0..10).flat_map(move |j| [i as f32, j as f32])).collect();
    let view = EventView::new(&data, 2, &channels).unwrap();
    let mut t = GateTree::new("All Events");
    let parent = t.add_gate(GateTree::ROOT, "P", Gate::rectangle("X", "Y", 1.5, 6.5, 1.5, 6.5).unwrap()).unwrap();
    let child = t.add_gate(parent, "C", Gate::rectangle("X", "Y", 2.5, 4.5, 2.5, 4.5).unwrap()).unwrap();
    let count = |t: &GateTree, n| t.evaluate(&view).unwrap().counts[n];
    assert_eq!((count(&t, parent), count(&t, child)), (25, 4));
    // shrink the parent so it cuts the child: the child's count follows because it is gated below
    t.set_gate(parent, Gate::rectangle("X", "Y", 1.5, 3.5, 1.5, 6.5).unwrap()).unwrap();
    assert_eq!(count(&t, parent), 10);
    assert_eq!(count(&t, child), 2, "x in {{3}} and y in {{3,4}} only");
    assert_eq!(t.name(parent).unwrap(), "P");
    assert_eq!(t.children(parent).unwrap(), &[child]);
    assert_eq!(t.path(child).unwrap(), vec!["P", "C"]);
    // a different shape, channel or scale is a different gate, not an edit
    assert!(t.set_gate(parent, Gate::interval("X", 1.0, 2.0).unwrap()).is_err());
    assert!(t.set_gate(parent, Gate::rectangle("Y", "X", 1.0, 2.0, 1.0, 2.0).unwrap()).is_err());
    // non-finite coordinates are refused and leave the gate as it was
    let mut bad = Gate::rectangle("X", "Y", 1.5, 3.5, 1.5, 6.5).unwrap();
    bad.xs[0] = f64::NAN;
    assert!(t.set_gate(parent, bad).is_err());
    assert!(matches!(t.kind(parent).unwrap(), NodeKind::Gate(g) if g.xs == vec![1.5, 3.5]));
    // the root has no geometry
    assert!(t.set_gate(GateTree::ROOT, Gate::rectangle("X", "Y", 0.0, 1.0, 0.0, 1.0).unwrap()).is_err());
}
