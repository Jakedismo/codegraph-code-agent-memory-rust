// ABOUTME: Durable semantic agent memory, independent of agent and MCP transports.
// ABOUTME: Owns typed policy, reconciliation, retrieval, and recoverable processing contracts.
pub mod context;
pub mod identity;
pub mod owner;
mod records;
pub mod service;
pub mod store;
pub mod types;
pub use service::{Classifier, Embedder, MemoryService};
pub use types::*;
