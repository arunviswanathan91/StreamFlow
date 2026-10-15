//! Gates, Boolean mask gates and the depth-independent gating tree for StreamFLOW.

pub mod discover;
pub mod gate;
pub mod gatingml;
pub mod mask;
pub mod tree;
pub mod workspace;
pub mod wsp;

pub use gate::{EdgeRule, EventView, Gate, GateError, GateKind, Quad};
pub use mask::Mask;
pub use tree::{BoolExpr, Evaluation, GateTree, NodeId, NodeKind, TreeError, Warning};
pub use workspace::{NodeMeta, SampleGates, WorkspaceError};
