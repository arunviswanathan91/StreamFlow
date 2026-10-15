//! The gating tree: populations as an arena of nodes, evaluated to per-event bitmasks.
//!
//! **Golden invariant (carried over from the Java app): nesting depth never matters.** There is no
//! recursion anywhere on the path from the tree to a population: parent chains, descendants, paths and
//! evaluation are all iterative, so a 10,000-level hierarchy behaves exactly like a 3-level one.
//!
//! Semantics:
//! * A **gate** node's population is `parent AND gate`.
//! * A **Boolean** node's population is `parent AND expr`, where `expr` combines the *absolute*
//!   populations of other nodes with AND / OR / NOT (FlowJo's "mask gates"); it normally hangs off the
//!   root so that `parent` is "all events".
//! * Sibling names are unique; adding a duplicate is an error and `replace_child` is the idempotent
//!   "apply gate to all" operation (BUG-15: applying must replace, never stack).
//! * A node that cannot be evaluated (missing channel, dangling Boolean reference) becomes an *empty
//!   population plus a warning*; it never aborts the evaluation of its siblings (BUG-01).

use crate::gate::{EventView, Gate, GateError};
use crate::mask::Mask;
use std::fmt;

pub type NodeId = usize;

/// Boolean expression over populations. Nesting is capped so evaluation (which is the only
/// recursive code here) cannot overflow the stack on hostile input.
#[derive(Debug, Clone, PartialEq)]
pub enum BoolExpr {
    Ref(NodeId),
    And(Vec<BoolExpr>),
    Or(Vec<BoolExpr>),
    Not(Box<BoolExpr>),
}

const MAX_EXPR_DEPTH: usize = 64;

impl BoolExpr {
    fn depth(&self) -> usize {
        match self {
            BoolExpr::Ref(_) => 1,
            BoolExpr::Not(e) => 1 + e.depth(),
            BoolExpr::And(v) | BoolExpr::Or(v) => 1 + v.iter().map(|e| e.depth()).max().unwrap_or(0),
        }
    }

    fn refs(&self, out: &mut Vec<NodeId>) {
        match self {
            BoolExpr::Ref(id) => out.push(*id),
            BoolExpr::Not(e) => e.refs(out),
            BoolExpr::And(v) | BoolExpr::Or(v) => v.iter().for_each(|e| e.refs(out)),
        }
    }

    fn eval(&self, masks: &[Option<Mask>], n: usize) -> Mask {
        match self {
            BoolExpr::Ref(id) => masks[*id].clone().expect("dependencies resolved before evaluation"),
            BoolExpr::Not(e) => e.eval(masks, n).not(),
            BoolExpr::And(v) => v.iter().fold(Mask::ones(n), |acc, e| acc.and(&e.eval(masks, n))),
            BoolExpr::Or(v) => v.iter().fold(Mask::zeros(n), |acc, e| acc.or(&e.eval(masks, n))),
        }
    }
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Root,
    Gate(Gate),
    Boolean(BoolExpr),
}

#[derive(Debug, Clone)]
struct Node {
    name: String,
    kind: NodeKind,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    alive: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TreeError {
    NoSuchNode(NodeId),
    DuplicateName { parent: String, name: String },
    BadExpression(String),
    Cycle(String),
    Gate(GateError),
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TreeError::NoSuchNode(id) => write!(f, "no such population (id {id})"),
            TreeError::DuplicateName { parent, name } => write!(f, "'{parent}' already has a population named '{name}'"),
            TreeError::BadExpression(m) => write!(f, "invalid Boolean gate: {m}"),
            TreeError::Cycle(n) => write!(f, "Boolean gates form a cycle through '{n}'"),
            TreeError::Gate(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TreeError {}

impl From<GateError> for TreeError {
    fn from(e: GateError) -> Self {
        TreeError::Gate(e)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub node: NodeId,
    pub name: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct Evaluation {
    /// Population mask per node id (`None` for removed nodes).
    pub masks: Vec<Option<Mask>>,
    pub counts: Vec<usize>,
    /// Percent of the parent population (NaN for the root or an empty parent).
    pub parent_pct: Vec<f64>,
    /// Percent of all events (NaN when there are no events).
    pub total_pct: Vec<f64>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone)]
pub struct GateTree {
    nodes: Vec<Node>,
}

impl GateTree {
    pub const ROOT: NodeId = 0;

    pub fn new(root_name: &str) -> GateTree {
        GateTree { nodes: vec![Node { name: root_name.to_string(), kind: NodeKind::Root, parent: None, children: vec![], alive: true }] }
    }

    fn node(&self, id: NodeId) -> Result<&Node, TreeError> {
        self.nodes.get(id).filter(|n| n.alive).ok_or(TreeError::NoSuchNode(id))
    }

    pub fn is_alive(&self, id: NodeId) -> bool {
        self.nodes.get(id).is_some_and(|n| n.alive)
    }

    /// Number of live populations, including the root.
    pub fn len(&self) -> usize {
        self.nodes.iter().filter(|n| n.alive).count()
    }

    pub fn is_empty(&self) -> bool {
        false // the root always exists
    }

    /// One past the largest node id ever issued (ids are never reused).
    pub fn id_bound(&self) -> usize {
        self.nodes.len()
    }

    /// Rename a population. The root cannot be renamed and siblings must stay uniquely named.
    pub fn rename(&mut self, id: NodeId, new_name: &str) -> Result<(), TreeError> {
        let parent = self.node(id)?.parent.ok_or_else(|| TreeError::BadExpression("the root cannot be renamed".into()))?;
        if new_name.trim().is_empty() {
            return Err(TreeError::BadExpression("a population needs a name".into()));
        }
        if self.nodes[parent].children.iter().any(|&c| c != id && self.nodes[c].name == new_name) {
            return Err(TreeError::DuplicateName { parent: self.nodes[parent].name.clone(), name: new_name.to_string() });
        }
        self.nodes[id].name = new_name.to_string();
        Ok(())
    }

    pub fn name(&self, id: NodeId) -> Result<&str, TreeError> {
        Ok(&self.node(id)?.name)
    }

    pub fn kind(&self, id: NodeId) -> Result<&NodeKind, TreeError> {
        Ok(&self.node(id)?.kind)
    }

    pub fn parent(&self, id: NodeId) -> Result<Option<NodeId>, TreeError> {
        Ok(self.node(id)?.parent)
    }

    pub fn children(&self, id: NodeId) -> Result<&[NodeId], TreeError> {
        Ok(&self.node(id)?.children)
    }

    pub fn depth(&self, id: NodeId) -> Result<usize, TreeError> {
        let (mut d, mut cur) = (0, self.node(id)?);
        while let Some(p) = cur.parent {
            d += 1;
            cur = &self.nodes[p];
        }
        Ok(d)
    }

    /// Names from just below the root down to `id` (empty for the root).
    pub fn path(&self, id: NodeId) -> Result<Vec<String>, TreeError> {
        let mut names = Vec::new();
        let mut cur = id;
        self.node(id)?;
        while let Some(p) = self.nodes[cur].parent {
            names.push(self.nodes[cur].name.clone());
            cur = p;
        }
        names.reverse();
        Ok(names)
    }

    /// Resolve a name path (below the root) to a population.
    pub fn find(&self, path: &[&str]) -> Option<NodeId> {
        let mut cur = Self::ROOT;
        for name in path {
            cur = *self.nodes[cur].children.iter().find(|&&c| self.nodes[c].name == *name)?;
        }
        Some(cur)
    }

    /// `id` and every population below it, parents before children.
    pub fn self_and_descendants(&self, id: NodeId) -> Result<Vec<NodeId>, TreeError> {
        self.node(id)?;
        let (mut out, mut stack) = (Vec::new(), vec![id]);
        while let Some(cur) = stack.pop() {
            out.push(cur);
            stack.extend(self.nodes[cur].children.iter().rev().copied());
        }
        Ok(out)
    }

    fn add(&mut self, parent: NodeId, name: &str, kind: NodeKind) -> Result<NodeId, TreeError> {
        let p = self.node(parent)?;
        if name.trim().is_empty() {
            return Err(TreeError::BadExpression("a population needs a name".into()));
        }
        if p.children.iter().any(|&c| self.nodes[c].name == name) {
            return Err(TreeError::DuplicateName { parent: p.name.clone(), name: name.to_string() });
        }
        let id = self.nodes.len();
        self.nodes.push(Node { name: name.to_string(), kind, parent: Some(parent), children: vec![], alive: true });
        self.nodes[parent].children.push(id);
        Ok(id)
    }

    pub fn add_gate(&mut self, parent: NodeId, name: &str, gate: Gate) -> Result<NodeId, TreeError> {
        self.add(parent, name, NodeKind::Gate(gate))
    }

    pub fn add_boolean(&mut self, parent: NodeId, name: &str, expr: BoolExpr) -> Result<NodeId, TreeError> {
        let d = expr.depth();
        if d > MAX_EXPR_DEPTH {
            return Err(TreeError::BadExpression(format!("expression nests {d} levels deep (limit {MAX_EXPR_DEPTH})")));
        }
        if let BoolExpr::And(v) | BoolExpr::Or(v) = &expr {
            if v.is_empty() {
                return Err(TreeError::BadExpression("AND/OR needs at least one operand".into()));
            }
        }
        let mut refs = Vec::new();
        expr.refs(&mut refs);
        if refs.is_empty() {
            return Err(TreeError::BadExpression("a Boolean gate must reference at least one population".into()));
        }
        for r in refs {
            if !self.is_alive(r) {
                return Err(TreeError::NoSuchNode(r));
            }
        }
        self.add(parent, name, NodeKind::Boolean(expr))
    }

    /// "Make And/Or Gate": the named populations combined.
    pub fn add_and(&mut self, parent: NodeId, name: &str, ids: &[NodeId]) -> Result<NodeId, TreeError> {
        self.add_boolean(parent, name, BoolExpr::And(ids.iter().map(|&i| BoolExpr::Ref(i)).collect()))
    }

    pub fn add_or(&mut self, parent: NodeId, name: &str, ids: &[NodeId]) -> Result<NodeId, TreeError> {
        self.add_boolean(parent, name, BoolExpr::Or(ids.iter().map(|&i| BoolExpr::Ref(i)).collect()))
    }

    /// "Make Not Gate".
    pub fn add_not(&mut self, parent: NodeId, name: &str, id: NodeId) -> Result<NodeId, TreeError> {
        self.add_boolean(parent, name, BoolExpr::Not(Box::new(BoolExpr::Ref(id))))
    }

    /// "Create Combination Gates": one And-gate per +/- pattern of `ids` (2^n of them, or 2^n - 1
    /// without the all-negative pattern). Named like `CD4+ & !CD8+`. The patterns are mutually
    /// exclusive and, when the all-negative one is included, exactly partition `parent`.
    pub fn add_combination_gates(&mut self, parent: NodeId, ids: &[NodeId], include_all_negative: bool) -> Result<Vec<NodeId>, TreeError> {
        let n = ids.len();
        if n == 0 || n > 12 {
            return Err(TreeError::BadExpression(format!("combination gates need 1 to 12 populations, got {n}")));
        }
        let names: Vec<String> = ids.iter().map(|&i| self.name(i).map(str::to_string)).collect::<Result<_, _>>()?;
        let mut out = Vec::new();
        for pattern in (0u32..(1 << n)).rev() {
            if pattern == 0 && !include_all_negative {
                continue;
            }
            let operands: Vec<BoolExpr> = (0..n)
                .map(|k| {
                    let positive = pattern >> (n - 1 - k) & 1 == 1;
                    if positive { BoolExpr::Ref(ids[k]) } else { BoolExpr::Not(Box::new(BoolExpr::Ref(ids[k]))) }
                })
                .collect();
            let label = (0..n)
                .map(|k| if pattern >> (n - 1 - k) & 1 == 1 { names[k].clone() } else { format!("!{}", names[k]) })
                .collect::<Vec<_>>()
                .join(" & ");
            out.push(self.add_boolean(parent, &label, BoolExpr::And(operands))?);
        }
        Ok(out)
    }

    /// Idempotent "apply to all": drop any same-named sibling (and its subtree), then add the gate.
    pub fn replace_gate(&mut self, parent: NodeId, name: &str, gate: Gate) -> Result<NodeId, TreeError> {
        if let Some(&old) = self.node(parent)?.children.iter().find(|&&c| self.nodes[c].name == name) {
            self.remove(old)?;
        }
        self.add_gate(parent, name, gate)
    }

    /// Remove a population and everything below it. The root cannot be removed. Ids are not reused.
    pub fn remove(&mut self, id: NodeId) -> Result<usize, TreeError> {
        if id == Self::ROOT {
            return Err(TreeError::BadExpression("the root population cannot be removed".into()));
        }
        let doomed = self.self_and_descendants(id)?;
        if let Some(p) = self.nodes[id].parent {
            self.nodes[p].children.retain(|&c| c != id);
        }
        for &d in &doomed {
            self.nodes[d].alive = false;
            self.nodes[d].children.clear();
        }
        Ok(doomed.len())
    }

    /// Everything a node's mask depends on: its parent and any Boolean references.
    fn dependencies(&self, id: NodeId) -> Vec<NodeId> {
        let n = &self.nodes[id];
        let mut deps: Vec<NodeId> = n.parent.into_iter().collect();
        if let NodeKind::Boolean(e) = &n.kind {
            e.refs(&mut deps);
        }
        deps
    }

    pub fn evaluate(&self, view: &EventView) -> Result<Evaluation, TreeError> {
        let n = view.n_events();
        let bound = self.nodes.len();
        let mut masks: Vec<Option<Mask>> = vec![None; bound];
        let mut state = vec![0u8; bound]; // 0 new, 1 on the DFS path, 2 done
        let mut warnings = Vec::new();

        for start in (0..bound).filter(|&i| self.nodes[i].alive) {
            let mut stack = vec![start];
            while let Some(&id) = stack.last() {
                if state[id] == 2 {
                    stack.pop();
                    continue;
                }
                if state[id] == 0 {
                    state[id] = 1;
                    for dep in self.dependencies(id) {
                        if !self.is_alive(dep) {
                            continue; // reported when this node is computed
                        }
                        match state[dep] {
                            1 => return Err(TreeError::Cycle(self.nodes[dep].name.clone())),
                            0 => stack.push(dep),
                            _ => {}
                        }
                    }
                    continue; // come back once the dependencies are done
                }
                // state 1 and every dependency is finished: compute this population
                let node = &self.nodes[id];
                let parent_mask = match node.parent {
                    Some(p) => masks[p].clone().expect("parent evaluated first"),
                    None => Mask::ones(n),
                };
                let own = match &node.kind {
                    NodeKind::Root => Mask::ones(n),
                    NodeKind::Gate(g) => match g.mask(view) {
                        Ok(m) => m,
                        Err(e) => {
                            warnings.push(Warning { node: id, name: node.name.clone(), message: e.to_string() });
                            Mask::zeros(n)
                        }
                    },
                    NodeKind::Boolean(e) => {
                        let mut refs = Vec::new();
                        e.refs(&mut refs);
                        if let Some(dead) = refs.iter().find(|&&r| !self.is_alive(r)) {
                            warnings.push(Warning { node: id, name: node.name.clone(), message: format!("references a deleted population (id {dead})") });
                            Mask::zeros(n)
                        } else {
                            e.eval(&masks, n)
                        }
                    }
                };
                masks[id] = Some(parent_mask.and(&own));
                state[id] = 2;
                stack.pop();
            }
        }

        let counts: Vec<usize> = masks.iter().map(|m| m.as_ref().map_or(0, Mask::count)).collect();
        let pct = |num: usize, den: usize| if den == 0 { f64::NAN } else { 100.0 * num as f64 / den as f64 };
        let parent_pct = (0..bound)
            .map(|i| match (self.nodes[i].alive, self.nodes[i].parent) {
                (true, Some(p)) => pct(counts[i], counts[p]),
                _ => f64::NAN,
            })
            .collect();
        let total_pct = (0..bound).map(|i| if self.nodes[i].alive { pct(counts[i], n) } else { f64::NAN }).collect();
        Ok(Evaluation { masks, counts, parent_pct, total_pct, warnings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::Gate;

    fn chans() -> Vec<String> {
        ["X", "Y"].iter().map(|s| s.to_string()).collect()
    }

    /// 100 events on a 10x10 grid: x = i % 10, y = i / 10 (both 0..9).
    fn grid() -> Vec<f32> {
        (0..100).flat_map(|i| [(i % 10) as f32, (i / 10) as f32]).collect()
    }

    fn rect(x0: f64, x1: f64, y0: f64, y1: f64) -> Gate {
        Gate::rectangle("X", "Y", x0, x1, y0, y1).unwrap().with_edges(crate::gate::EdgeRule::Inclusive)
    }

    #[test]
    fn counts_percentages_and_paths_are_hand_checkable() {
        let (data, c) = (grid(), chans());
        let v = EventView::new(&data, 2, &c).unwrap();
        let mut t = GateTree::new("All Events");
        let a = t.add_gate(GateTree::ROOT, "A", rect(0.0, 4.0, 0.0, 9.0)).unwrap(); // x 0..4 -> 50 events
        let b = t.add_gate(a, "B", rect(0.0, 9.0, 0.0, 1.0)).unwrap(); // y 0..1 -> 2 rows x 5 = 10
        let ev = t.evaluate(&v).unwrap();
        assert_eq!((ev.counts[0], ev.counts[a], ev.counts[b]), (100, 50, 10));
        assert_eq!((ev.parent_pct[a], ev.parent_pct[b]), (50.0, 20.0));
        assert_eq!((ev.total_pct[a], ev.total_pct[b]), (50.0, 10.0));
        assert!(ev.parent_pct[0].is_nan());
        assert_eq!(t.path(b).unwrap(), vec!["A", "B"]);
        assert_eq!(t.find(&["A", "B"]), Some(b));
        assert_eq!(t.depth(b).unwrap(), 2);
        assert!(ev.warnings.is_empty());
    }

    #[test]
    fn golden_invariant_any_nesting_depth_opens_and_evaluates() {
        // the documented 5-level case, then a depth that would overflow any recursive implementation
        let (data, c) = (grid(), chans());
        let v = EventView::new(&data, 2, &c).unwrap();
        let mut t = GateTree::new("All Events");
        let mut cur = GateTree::ROOT;
        for (i, name) in ["Lymphocytes", "CD3+", "CD4+", "CD25+", "FoxP3+"].iter().enumerate() {
            cur = t.add_gate(cur, name, rect(0.0, 9.0 - i as f64, 0.0, 9.0)).unwrap();
        }
        let ev = t.evaluate(&v).unwrap();
        assert_eq!(t.depth(cur).unwrap(), 5);
        // the x upper bounds are 9, 8, 7, 6, 5 so the survivors are x in 0..=5: 6 columns x 10 rows
        assert_eq!(ev.counts[cur], 60);
    }

    #[test]
    fn a_very_deep_chain_neither_overflows_nor_slows_to_a_crawl() {
        let (data, c) = (vec![1.0f32, 1.0, 2.0, 2.0], chans());
        let v = EventView::new(&data, 2, &c).unwrap();
        let mut t = GateTree::new("root");
        let mut cur = GateTree::ROOT;
        for i in 0..10_000 {
            cur = t.add_gate(cur, &format!("L{i}"), rect(0.0, 5.0, 0.0, 5.0)).unwrap();
        }
        assert_eq!(t.depth(cur).unwrap(), 10_000);
        assert_eq!(t.path(cur).unwrap().len(), 10_000);
        assert_eq!(t.self_and_descendants(GateTree::ROOT).unwrap().len(), 10_001);
        let ev = t.evaluate(&v).unwrap();
        assert_eq!(ev.counts[cur], 2);
        assert_eq!(t.remove(1).unwrap(), 10_000);
    }

    #[test]
    fn a_cycle_in_corrupted_data_is_an_error_not_a_hang() {
        let (data, c) = (grid(), chans());
        let v = EventView::new(&data, 2, &c).unwrap();
        let mut t = GateTree::new("All");
        let a = t.add_gate(GateTree::ROOT, "A", rect(0.0, 4.0, 0.0, 9.0)).unwrap();
        let b = t.add_and(GateTree::ROOT, "B", &[a]).unwrap();
        // corrupt: make A a Boolean that refers to B, which refers to A
        t.nodes[a].kind = NodeKind::Boolean(BoolExpr::Ref(b));
        assert!(matches!(t.evaluate(&v), Err(TreeError::Cycle(_))));
    }
}
