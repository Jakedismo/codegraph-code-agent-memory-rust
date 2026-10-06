// ABOUTME: Content-free project memory projection contracts shared by durable and code stores.
// ABOUTME: Cursors and exact revisions make rebuildable graph candidates safe to revalidate.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionCursor {
    pub generation: u64,
    pub memory_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionAnchor {
    pub graph_project_id: String,
    pub node_id: String,
    pub supporting: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionChange {
    pub memory_id: String,
    pub revision: u64,
    pub generation: u64,
    pub enabled: bool,
    pub anchors: Vec<ProjectionAnchor>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionBatch {
    pub source_id: String,
    pub cursor: ProjectionCursor,
    pub complete: bool,
    pub changes: Vec<ProjectionChange>,
}
