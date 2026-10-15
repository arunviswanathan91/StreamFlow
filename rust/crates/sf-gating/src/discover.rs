//! From clusters to populations: the gating half of cluster discovery.
//!
//! A cluster is only ever a *proposal* until it is approved; approving turns its events into an index-set
//! population in the gating tree (exactly what the app does with an approved FlowSOM suggestion). Unlike
//! the app, the rows here are file rows, so a cluster trained on a downsample can cover the whole file
//! (see `FlowSom::predict`). Approving the same name again replaces the population instead of stacking a
//! duplicate (BUG-15), and children can be gated inside a cluster population at any depth.

use crate::{Gate, GateTree, NodeId, TreeError};

/// Rows (file event indices) of every cluster `0..k`; events with no label (`None`) belong to none.
pub fn cluster_rows(labels: &[Option<usize>], k: usize) -> Vec<Vec<u32>> {
    let mut rows = vec![Vec::new(); k];
    for (i, l) in labels.iter().enumerate() {
        if let Some(c) = l {
            if *c < k {
                rows[*c].push(i as u32);
            }
        }
    }
    rows
}

/// Approve one cluster as a population named `name` under `parent`. Replaces an existing sibling of the
/// same name. Returns the node id.
pub fn approve_cluster(tree: &mut GateTree, parent: NodeId, name: &str, rows: Vec<u32>) -> Result<NodeId, TreeError> {
    tree.replace_gate(parent, name, Gate::indexed(rows))
}

/// Approve every non-empty cluster under `parent`, named by `names[c]`. Duplicate names get a ` (c+1)` suffix
/// so two clusters with the same phenotype name cannot overwrite each other.
pub fn approve_all(tree: &mut GateTree, parent: NodeId, names: &[String], rows: Vec<Vec<u32>>) -> Result<Vec<NodeId>, TreeError> {
    assert_eq!(names.len(), rows.len(), "one name per cluster");
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (c, r) in rows.into_iter().enumerate() {
        if r.is_empty() {
            continue;
        }
        let mut name = names[c].clone();
        if !used.insert(name.clone()) {
            name = format!("{} ({})", names[c], c + 1);
            used.insert(name.clone());
        }
        out.push(approve_cluster(tree, parent, &name, r)?);
    }
    Ok(out)
}
