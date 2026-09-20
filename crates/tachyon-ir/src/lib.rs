//! Tachyon Ir.
//!
//! Validated, machine-readable execution graphs (spec §5–§8).
//!
//! Milestone 1 scope: graph shape (nodes, dependencies), cycle and
//! reference validation. Executor kinds, invocations, access/resource
//! claims, and retry/cancellation/verification metadata arrive in
//! Milestone 2, which extends [`ExecutionNode`] without changing the
//! validation guarantees established here.

#![warn(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use tachyon_types::{NodeId, TaskId};

/// IR schema version. Bump on any breaking graph change.
pub const IR_VERSION: u16 = 1;

/// Errors produced while validating an [`ExecutionGraph`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IrError {
    /// A dependency names a node absent from [`ExecutionGraph::nodes`].
    #[error("dependency references unknown node {0}")]
    UnknownNode(NodeId),
    /// Dependencies form a cycle; the graph is not a DAG.
    #[error("dependency cycle detected")]
    Cycle,
    /// A node belongs to a different task than the graph.
    #[error("node {0} belongs to another task")]
    ForeignTask(NodeId),
}

/// One scheduled operation. Milestone 2 adds executor, invocation,
/// access/effect/resource metadata; the identity fields here are stable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionNode {
    /// Unique node identity.
    pub id: NodeId,
    /// Task that owns this node.
    pub task_id: TaskId,
    /// Task revision this node was planned against (spec §5).
    pub planned_revision: u64,
}

/// Ordering edge: `to` runs only after `from` completes as required.
/// Dependency conditions (`OnSuccess`/`OnFailure`/`OnCompletion`) arrive
/// in Milestone 2; until then every edge means success-required.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// Upstream node.
    pub from: NodeId,
    /// Downstream node.
    pub to: NodeId,
}

/// A validated, dependency-aware execution graph (spec §5).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionGraph {
    /// Schema version; always [`IR_VERSION`] for graphs built here.
    pub version: u16,
    /// Nodes by id.
    pub nodes: BTreeMap<NodeId, ExecutionNode>,
    /// Ordering edges.
    pub dependencies: Vec<Dependency>,
}

impl ExecutionGraph {
    /// Creates an empty graph for `task_id` at `revision`.
    #[must_use]
    pub fn empty(_task_id: TaskId, _revision: u64) -> Self {
        Self {
            version: IR_VERSION,
            nodes: BTreeMap::new(),
            dependencies: Vec::new(),
        }
    }

    /// Validates structural invariants: node/task ownership, reference
    /// existence, and acyclicity.
    pub fn validate(&self, task_id: TaskId) -> Result<(), IrError> {
        for node in self.nodes.values() {
            if node.task_id != task_id {
                return Err(IrError::ForeignTask(node.id));
            }
        }
        for edge in &self.dependencies {
            if !self.nodes.contains_key(&edge.from) {
                return Err(IrError::UnknownNode(edge.from));
            }
            if !self.nodes.contains_key(&edge.to) {
                return Err(IrError::UnknownNode(edge.to));
            }
        }
        if has_cycle(&self.nodes, &self.dependencies) {
            return Err(IrError::Cycle);
        }
        Ok(())
    }
}

/// Iterative depth-first cycle detection over `from -> to` edges.
fn has_cycle(nodes: &BTreeMap<NodeId, ExecutionNode>, edges: &[Dependency]) -> bool {
    let mut outgoing: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for edge in edges {
        outgoing.entry(edge.from).or_default().push(edge.to);
    }
    let mut visited: BTreeSet<NodeId> = BTreeSet::new();
    let mut visiting: BTreeSet<NodeId> = BTreeSet::new();
    for root in nodes.keys() {
        if visited.contains(root) {
            continue;
        }
        let mut stack: Vec<(NodeId, bool)> = vec![(*root, false)];
        while let Some((node, exiting)) = stack.pop() {
            if exiting {
                visiting.remove(&node);
                continue;
            }
            if !visited.insert(node) {
                if visiting.contains(&node) {
                    return true;
                }
                continue;
            }
            visiting.insert(node);
            stack.push((node, true));
            if let Some(next) = outgoing.get(&node) {
                for child in next {
                    if !visited.contains(child) || visiting.contains(child) {
                        stack.push((*child, false));
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{Dependency, ExecutionNode};
    use super::{ExecutionGraph, IrError};
    use tachyon_types::{NodeId, TaskId};

    fn node(task: TaskId) -> ExecutionNode {
        ExecutionNode {
            id: NodeId::generate(),
            task_id: task,
            planned_revision: 0,
        }
    }

    #[test]
    fn empty_graph_is_valid() {
        let task = TaskId::generate();
        let graph = ExecutionGraph::empty(task, 0);
        assert_eq!(graph.validate(task), Ok(()));
    }

    #[test]
    fn linear_chain_is_valid() {
        let task = TaskId::generate();
        let first = node(task);
        let second = node(task);
        let graph = super::ExecutionGraph {
            version: super::IR_VERSION,
            nodes: [first.clone(), second.clone()]
                .into_iter()
                .map(|n| (n.id, n))
                .collect(),
            dependencies: vec![Dependency {
                from: first.id,
                to: second.id,
            }],
        };
        assert_eq!(graph.validate(task), Ok(()));
    }

    #[test]
    fn cycle_is_rejected() {
        let task = TaskId::generate();
        let first = node(task);
        let second = node(task);
        let graph = super::ExecutionGraph {
            version: super::IR_VERSION,
            nodes: [first.clone(), second.clone()]
                .into_iter()
                .map(|n| (n.id, n))
                .collect(),
            dependencies: vec![
                Dependency {
                    from: first.id,
                    to: second.id,
                },
                Dependency {
                    from: second.id,
                    to: first.id,
                },
            ],
        };
        assert_eq!(graph.validate(task), Err(IrError::Cycle));
    }

    #[test]
    fn dangling_and_foreign_nodes_are_rejected() {
        let task = TaskId::generate();
        let first = node(task);
        let graph = super::ExecutionGraph {
            version: super::IR_VERSION,
            nodes: [(first.id, first.clone())].into_iter().collect(),
            dependencies: vec![Dependency {
                from: first.id,
                to: NodeId::generate(),
            }],
        };
        assert!(matches!(graph.validate(task), Err(IrError::UnknownNode(_))));

        let foreign = node(TaskId::generate());
        let graph = super::ExecutionGraph {
            version: super::IR_VERSION,
            nodes: [(foreign.id, foreign.clone())].into_iter().collect(),
            dependencies: vec![],
        };
        assert_eq!(graph.validate(task), Err(IrError::ForeignTask(foreign.id)));
    }
}
