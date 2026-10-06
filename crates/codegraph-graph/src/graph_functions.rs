// ABOUTME: Rust SDK wrappers for SurrealDB graph analysis functions
// ABOUTME: Provides type-safe interfaces for LLM-powered graph analysis tools

use codegraph_core::{CodeGraphError, Result};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use std::sync::Arc;
use surrealdb::types::{SurrealValue as _, ToSql, Value as SurrealValue};
use surrealdb::{Surreal, engine::any::Any};
use tracing::{debug, error, warn};

/// Convert SurrealDB Value to clean serde_json::Value using accessor methods
/// Avoids externally-tagged enum serialization that produces {"None": ...}, {"Number": {"Int": ...}}
fn surreal_to_json(value: SurrealValue) -> serde_json::Value {
    sql_value_to_json(value)
}

fn sql_value_to_json(value: surrealdb::types::Value) -> serde_json::Value {
    use surrealdb::types::Value as SqlValue;
    match value {
        SqlValue::None | SqlValue::Null => serde_json::Value::Null,
        SqlValue::Bool(b) => serde_json::Value::Bool(b),
        SqlValue::Number(n) => surrealdb::types::Value::Number(n).into_json_value(),
        SqlValue::Duration(d) => serde_json::Value::String(d.to_string()),
        SqlValue::Datetime(dt) => serde_json::Value::String(dt.to_string()),
        SqlValue::Uuid(u) => serde_json::Value::String(u.to_string()),
        SqlValue::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(sql_value_to_json).collect())
        }
        SqlValue::Object(obj) => {
            let map: serde_json::Map<String, serde_json::Value> = obj
                .into_iter()
                .map(|(k, v)| (k.to_string(), sql_value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
        SqlValue::RecordId(thing) => serde_json::Value::String(thing.to_sql()),
        SqlValue::Bytes(b) => serde_json::Value::String(format!("bytes:{}", b.len())),
        SqlValue::String(s) => {
            let s_str = s.to_string();
            // Transparently decompress if it looks like our compressed format
            match codegraph_core::decompress_string(&s_str) {
                Ok(decompressed) => {
                    // Try parsing as JSON first if it looks like it
                    if (decompressed.starts_with('{') && decompressed.ends_with('}'))
                        || (decompressed.starts_with('[') && decompressed.ends_with(']'))
                    {
                        if let Ok(json) = serde_json::from_str(&decompressed) {
                            return json;
                        }
                    }
                    serde_json::Value::String(decompressed)
                }
                _ => serde_json::Value::String(s_str),
            }
        }
        other => serde_json::Value::String(other.to_sql()),
    }
}

/// Wrapper for SurrealDB graph analysis functions
/// Provides type-safe Rust interfaces for calling SurrealDB functions
#[derive(Clone)]
pub struct GraphFunctions {
    db: Arc<Surreal<Any>>,
    project_id: String,
}

impl GraphFunctions {
    /// Detect an available embedding dimension by probing chunk columns.
    #[allow(dead_code)]
    async fn detect_embedding_dimension(&self) -> Result<usize> {
        let sql = r#"
            RETURN {
                has_1024: (SELECT VALUE count() FROM chunks WHERE project_id = $project_id AND embedding_1024 != NONE LIMIT 1)[0],
                has_768:  (SELECT VALUE count() FROM chunks WHERE project_id = $project_id AND embedding_768  != NONE LIMIT 1)[0],
                has_384:  (SELECT VALUE count() FROM chunks WHERE project_id = $project_id AND embedding_384  != NONE LIMIT 1)[0]
            };
        "#;
        let mut resp = self
            .db
            .query(sql)
            .bind(("project_id", self.project_id.clone()))
            .await
            .map_err(|e| CodeGraphError::Database(format!("dimension probe failed: {e}")))?;
        let val: Option<SurrealValue> = resp.take(0).ok();
        let json = val
            .map(surreal_to_json)
            .unwrap_or_else(|| serde_json::Value::Null);
        let has_1024 = json.get("has_1024").and_then(|v| v.as_u64()).unwrap_or(0);
        let has_768 = json.get("has_768").and_then(|v| v.as_u64()).unwrap_or(0);
        let has_384 = json.get("has_384").and_then(|v| v.as_u64()).unwrap_or(0);
        if has_1024 > 0 {
            Ok(1024)
        } else if has_768 > 0 {
            Ok(768)
        } else if has_384 > 0 {
            Ok(384)
        } else {
            Err(CodeGraphError::Database(
                "No embeddings found for this project".to_string(),
            ))
        }
    }

    pub fn new(db: Arc<Surreal<Any>>) -> Self {
        Self {
            db,
            project_id: Self::default_project_id(),
        }
    }

    pub fn new_with_project_id(db: Arc<Surreal<Any>>, project_id: impl Into<String>) -> Self {
        Self {
            db,
            project_id: Self::normalize_project_id(project_id.into()),
        }
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// Local projection cursors reset when the rebuildable code snapshot changes.
    pub async fn memory_projection_cursor(
        &self,
        source_id: &str,
    ) -> Result<codegraph_core::memory_projection::ProjectionCursor> {
        use codegraph_core::memory_projection::ProjectionCursor;
        self.db
            .query(include_str!(
                "../../../schema/agent_memory_projection_v1.surql"
            ))
            .await
            .map_err(|e| CodeGraphError::Database(e.to_string()))?
            .check()
            .map_err(|e| CodeGraphError::Database(e.to_string()))?;
        self.db.query(
            "BEGIN TRANSACTION; LET $fingerprint=(SELECT VALUE metadata.input_fingerprint FROM project_metadata WHERE project_id=$project LIMIT 1)[0]; LET $previous=(SELECT * FROM memory_projection_cursor WHERE source_id=$source AND project_id=$project LIMIT 1)[0]; IF $previous != NONE AND $previous.code_input_fingerprint != $fingerprint { DELETE memory_code_anchor WHERE source_id=$source AND project_id=$project; DELETE memory_projection WHERE source_id=$source AND project_id=$project; DELETE memory_projection_cursor WHERE source_id=$source AND project_id=$project; }; COMMIT TRANSACTION;"
        ).bind(("project",self.project_id.clone())).bind(("source",source_id.to_string())).await
            .map_err(|e|CodeGraphError::Database(e.to_string()))?.check()
            .map_err(|e|CodeGraphError::Database(e.to_string()))?;
        let rows:Vec<serde_json::Value>=self.db.query("SELECT generation,memory_id FROM memory_projection_cursor WHERE source_id=$source AND project_id=$project LIMIT 1;")
            .bind(("project",self.project_id.clone())).bind(("source",source_id.to_string())).await
            .map_err(|e|CodeGraphError::Database(e.to_string()))?.check()
            .map_err(|e|CodeGraphError::Database(e.to_string()))?.take(0)
            .map_err(|e|CodeGraphError::Database(e.to_string()))?;
        match rows.into_iter().next() {
            Some(row) => serde_json::from_value(row)
                .map_err(|e| CodeGraphError::Database(format!("Invalid projection cursor: {e}"))),
            None => Ok(ProjectionCursor::default()),
        }
    }

    /// Native relations point only to existing local code nodes; no memory text is copied.
    pub async fn apply_memory_projection(
        &self,
        batch: &codegraph_core::memory_projection::ProjectionBatch,
        expected: &codegraph_core::memory_projection::ProjectionCursor,
    ) -> Result<()> {
        use sha2::{Digest, Sha256};
        if batch.changes.len() > 256 {
            return Err(CodeGraphError::Database(
                "Unbounded memory projection batch".into(),
            ));
        }
        let key = |text: &str| -> String {
            Sha256::digest(text.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        };
        let mut changes = Vec::new();
        for change in &batch.changes {
            let record = key(&format!("{}:{}", batch.source_id, change.memory_id));
            let anchors=change.anchors.iter().filter(|anchor|anchor.graph_project_id==self.project_id).map(|anchor| {
                let node=if anchor.node_id.starts_with("nodes:"){anchor.node_id.clone()}else{format!("nodes:{}",anchor.node_id)};
                serde_json::json!({"node":node,"edge":key(&format!("{}:{node}",record)),"role":if anchor.supporting{"evidence"}else{"related"}})
            }).collect::<Vec<_>>();
            changes.push(serde_json::json!({"record":record,"memory_id":change.memory_id,"revision":change.revision,"enabled":change.enabled,"anchors":anchors}));
        }
        self.db.query(
            "BEGIN TRANSACTION;
             LET $old=(SELECT * FROM memory_projection_cursor WHERE source_id=$source AND project_id=$project LIMIT 1)[0];
             LET $generation=$old.generation ?? 0; LET $memory_id=$old.memory_id ?? '';
             IF $generation != $expected_generation OR $memory_id != $expected_id { THROW 'Memory projection cursor conflict'; };
             FOR $change IN $changes {
                LET $record=type::record('memory_projection',$change.record);
                DELETE memory_code_anchor WHERE source_id=$source AND project_id=$project AND in=$record;
                IF $change.enabled {
                    UPSERT $record CONTENT {source_id:$source,project_id:$project,memory_id:$change.memory_id,revision:$change.revision};
                    FOR $anchor IN $change.anchors {
                        LET $node=type::record($anchor.node);
                        LET $found=(SELECT VALUE id FROM nodes WHERE project_id=$project AND id=$node LIMIT 1)[0];
                        IF $found != NONE {
                            LET $edge=type::record('memory_code_anchor',$anchor.edge);
                            RELATE $record->$edge->$node CONTENT {source_id:$source,project_id:$project,role:$anchor.role};
                        };
                    };
                } ELSE { DELETE $record; };
             };
             LET $fingerprint=(SELECT VALUE metadata.input_fingerprint FROM project_metadata WHERE project_id=$project LIMIT 1)[0];
             LET $cursor=type::record('memory_projection_cursor',$source);
             UPSERT $cursor CONTENT {source_id:$source,project_id:$project,generation:$generation_next,memory_id:$memory_id_next,code_input_fingerprint:$fingerprint};
             COMMIT TRANSACTION;"
        ).bind(("source",batch.source_id.clone())).bind(("project",self.project_id.clone())).bind(("changes",changes))
            .bind(("expected_generation",expected.generation)).bind(("expected_id",expected.memory_id.clone()))
            .bind(("generation_next",batch.cursor.generation)).bind(("memory_id_next",batch.cursor.memory_id.clone())).await
            .map_err(|e|CodeGraphError::Database(e.to_string()))?.check()
            .map_err(|e|CodeGraphError::Database(e.to_string()))?;
        Ok(())
    }

    /// Scope-bounded native joins between projection references, anchors, nodes and code edges.
    pub async fn memory_projection_candidates(
        &self,
        source_id: &str,
        node_ids: &[String],
    ) -> Result<Vec<serde_json::Value>> {
        let ids = node_ids.iter().take(32).cloned().collect::<Vec<_>>();
        let mut response=self.db.query(
            "LET $roots=SELECT VALUE id FROM nodes WHERE project_id=$project AND type::string(id) INSIDE $ids LIMIT 32;
             LET $edges=SELECT id,from,to,edge_type,metadata FROM edges WHERE project_id=$project AND (from INSIDE $roots OR to INSIDE $roots) LIMIT 64;
             LET $neighbors=SELECT VALUE id FROM nodes WHERE project_id=$project AND (id INSIDE $edges.from OR id INSIDE $edges.to) LIMIT 32;
             LET $candidates=SELECT in.memory_id AS memory_id,in.revision AS revision,type::string(out) AS node_id,type::string(id) AS anchor_id,role AS anchor_role,IF out INSIDE $roots THEN 0 ELSE 1 END AS code_hop_count FROM memory_code_anchor WHERE project_id=$project AND source_id=$source AND (out INSIDE $roots OR out INSIDE $neighbors) ORDER BY code_hop_count ASC,memory_id ASC LIMIT 100;
             RETURN {candidates:$candidates,roots:$roots,edges:$edges};"
        ).bind(("source",source_id.to_string())).bind(("project",self.project_id.clone())).bind(("ids",ids)).await
            .map_err(|e|CodeGraphError::Database(e.to_string()))?.check()
            .map_err(|e|CodeGraphError::Database(e.to_string()))?;
        let raw: surrealdb::types::Value = response
            .take(response.num_statements() - 1)
            .map_err(|e| CodeGraphError::Database(e.to_string()))?;
        let context = surreal_to_json(raw);
        let roots = context["roots"].as_array().cloned().unwrap_or_default();
        let edges = context["edges"].as_array().cloned().unwrap_or_default();
        let mut candidates = context["candidates"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for candidate in &mut candidates {
            let node = candidate["node_id"].clone();
            let mut paths = Vec::new();
            if candidate["code_hop_count"] == 0 {
                paths.push(serde_json::json!({"kind":"retrieval","matched_node":node,"anchor_node":node,"code_hop_count":0,"memory_anchor":candidate["anchor_id"],"anchor_role":candidate["anchor_role"]}));
            } else {
                for edge in &edges {
                    let from = &edge["from"];
                    let to = &edge["to"];
                    let matched = if *from == node && roots.contains(to) {
                        Some((to, "incoming"))
                    } else if *to == node && roots.contains(from) {
                        Some((from, "outgoing"))
                    } else {
                        None
                    };
                    if let Some((matched, direction)) = matched {
                        paths.push(serde_json::json!({"kind":"retrieval","matched_node":matched,"anchor_node":node,"code_hop_count":1,"direction":direction,"code_edge":edge,"memory_anchor":candidate["anchor_id"],"anchor_role":candidate["anchor_role"]}));
                    }
                }
            }
            candidate["paths_truncated"] = serde_json::json!(paths.len() > 4);
            paths.truncate(4);
            candidate["paths"] = serde_json::json!(paths);
        }
        Ok(candidates)
    }

    /// Expose Surreal DB handle (for diagnostics/tests only)
    pub fn db(&self) -> Arc<Surreal<Any>> {
        self.db.clone()
    }

    /// Expose raw DB for diagnostics/tests (not for production use).
    #[cfg(test)]
    pub fn raw_db(&self) -> Arc<Surreal<Any>> {
        self.db.clone()
    }

    /// Bounded one-hop code context for optional memory anchors; never crosses project scope.
    pub async fn memory_code_context(&self, node_ids: &[String]) -> Result<serde_json::Value> {
        let ids: Vec<String> = node_ids
            .iter()
            .take(32)
            .map(|id| {
                if id.starts_with("nodes:") {
                    id.clone()
                } else {
                    format!("nodes:{id}")
                }
            })
            .collect();
        let mut response = self.db.query(
            "LET $roots = SELECT * FROM nodes WHERE project_id = $project AND type::string(id) INSIDE $ids LIMIT 32; LET $edges = SELECT * FROM edges WHERE project_id = $project AND (from INSIDE $roots.id OR to INSIDE $roots.id) LIMIT 64; LET $nodes = SELECT id,name,file_path,start_line,end_line,content,metadata FROM nodes WHERE project_id = $project AND (id INSIDE $roots.id OR id INSIDE $edges.from OR id INSIDE $edges.to) LIMIT 32; RETURN { nodes: $nodes, edges: $edges, files: (SELECT file_path,content_hash FROM file_metadata WHERE project_id = $project AND file_path INSIDE $nodes.file_path LIMIT 32), graph_complete: (SELECT VALUE metadata.stats.graph_complete FROM project_metadata WHERE project_id = $project LIMIT 1)[0], input_fingerprint: (SELECT VALUE metadata.input_fingerprint FROM project_metadata WHERE project_id = $project LIMIT 1)[0] };"
        ).bind(("project",self.project_id.clone())).bind(("ids",ids)).await
            .map_err(|e|CodeGraphError::Database(e.to_string()))?.check()
            .map_err(|e|CodeGraphError::Database(e.to_string()))?;
        let final_statement = response.num_statements().saturating_sub(1);
        let value: SurrealValue = response
            .take(final_statement)
            .map_err(|e| CodeGraphError::Database(e.to_string()))?;
        Ok(surreal_to_json(value))
    }

    fn default_project_id() -> String {
        let env_value = std::env::var("CODEGRAPH_PROJECT_ID")
            .ok()
            .filter(|v| !v.trim().is_empty());

        let inferred = env_value.or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        });

        let raw = inferred.unwrap_or_else(|| "default-project".to_string());
        Self::normalize_project_id(raw)
    }

    /// Normalize project_id to reduce mismatches (canonical path where possible, trimmed slashes)
    fn normalize_project_id(raw: String) -> String {
        use std::path::Path;
        let trimmed = raw.trim().trim_end_matches('/').to_string();
        if trimmed.is_empty() {
            return "default-project".to_string();
        }
        if Path::new(&trimmed).exists() {
            if let Ok(canon) = std::fs::canonicalize(&trimmed) {
                return canon.display().to_string();
            }
        }
        trimmed
    }

    /// Clone with an explicit project_id override
    pub fn with_project_id(&self, project_id: impl Into<String>) -> Self {
        Self {
            db: self.db.clone(),
            project_id: Self::normalize_project_id(project_id.into()),
        }
    }

    /// Get transitive dependencies of a node up to specified depth
    ///
    /// # Arguments
    /// * `node_id` - The ID of the node to analyze
    /// * `edge_type` - The type of edge to follow (e.g., "Calls", "Imports")
    /// * `depth` - Maximum depth to traverse (1-10, defaults to 3)
    ///
    /// # Returns
    /// Vector of nodes representing transitive dependencies
    pub async fn get_transitive_dependencies(
        &self,
        node_id: &str,
        edge_type: &str,
        depth: i32,
    ) -> Result<Vec<DependencyNode>> {
        debug!(
            "Calling fn::get_transitive_dependencies({}, {}, {}, project={})",
            node_id, edge_type, depth, self.project_id
        );

        let result: Vec<DependencyNode> = self
            .db
            .query(
                "RETURN fn::get_transitive_dependencies($project_id, $node_id, $edge_type, $depth)",
            )
            .bind(("project_id", self.project_id.clone()))
            .bind(("node_id", node_id.to_string()))
            .bind(("edge_type", edge_type.to_string()))
            .bind(("depth", depth))
            .await
            .map_err(|e| {
                error!("Failed to call get_transitive_dependencies: {}", e);
                CodeGraphError::Database(format!("get_transitive_dependencies failed: {}", e))
            })?
            .take(0)
            .map_err(|e| {
                error!("Failed to deserialize dependencies: {}", e);
                CodeGraphError::Database(format!("Deserialization failed: {}", e))
            })?;

        Ok(result)
    }

    /// Detect circular dependencies for a given edge type
    ///
    /// # Arguments
    /// * `edge_type` - The type of edge to analyze (e.g., "Imports", "Uses")
    ///
    /// # Returns
    /// Vector of circular dependency pairs (A <-> B relationships)
    pub async fn detect_circular_dependencies(
        &self,
        edge_type: &str,
    ) -> Result<Vec<CircularDependency>> {
        debug!(
            "Calling fn::detect_circular_dependencies({}, project={})",
            edge_type, self.project_id
        );

        let result: Vec<CircularDependency> = self
            .db
            .query("RETURN fn::detect_circular_dependencies($project_id, $edge_type)")
            .bind(("project_id", self.project_id.clone()))
            .bind(("edge_type", edge_type.to_string()))
            .await
            .map_err(|e| {
                error!("Failed to call detect_circular_dependencies: {}", e);
                CodeGraphError::Database(format!("detect_circular_dependencies failed: {}", e))
            })?
            .take(0)
            .map_err(|e| {
                error!("Failed to deserialize circular dependencies: {}", e);
                CodeGraphError::Database(format!("Deserialization failed: {}", e))
            })?;

        Ok(result)
    }

    /// Trace the call chain starting from a node
    ///
    /// # Arguments
    /// * `from_node` - The ID of the starting node
    /// * `max_depth` - Maximum depth to traverse (1-10, defaults to 5)
    ///
    /// # Returns
    /// Vector of nodes in the call chain with depth information
    pub async fn trace_call_chain(
        &self,
        from_node: &str,
        max_depth: i32,
    ) -> Result<Vec<CallChainNode>> {
        debug!(
            "Calling fn::trace_call_chain({}, {}, project={})",
            from_node, max_depth, self.project_id
        );

        let result: Vec<CallChainNode> = self
            .db
            .query("RETURN fn::trace_call_chain($project_id, $from_node, $max_depth)")
            .bind(("project_id", self.project_id.clone()))
            .bind(("from_node", from_node.to_string()))
            .bind(("max_depth", max_depth))
            .await
            .map_err(|e| {
                error!("Failed to call trace_call_chain: {}", e);
                CodeGraphError::Database(format!("trace_call_chain failed: {}", e))
            })?
            .take(0)
            .map_err(|e| {
                error!("Failed to deserialize call chain: {}", e);
                CodeGraphError::Database(format!("Deserialization failed: {}", e))
            })?;

        Ok(result)
    }

    /// Calculate coupling metrics for a node
    ///
    /// # Arguments
    /// * `node_id` - The ID of the node to analyze
    ///
    /// # Returns
    /// Coupling metrics including afferent, efferent, and instability
    pub async fn calculate_coupling_metrics(&self, node_id: &str) -> Result<CouplingMetricsResult> {
        debug!(
            "Calling fn::calculate_coupling_metrics({}, project={})",
            node_id, self.project_id
        );

        // Use Option to handle NONE returned by SurrealDB when node doesn't exist
        let results: Vec<Option<CouplingMetricsResult>> = self
            .db
            .query("RETURN fn::calculate_coupling_metrics($project_id, $node_id)")
            .bind(("project_id", self.project_id.clone()))
            .bind(("node_id", node_id.to_string()))
            .await
            .map_err(|e| {
                error!("Failed to call calculate_coupling_metrics: {}", e);
                CodeGraphError::Database(format!("calculate_coupling_metrics failed: {}", e))
            })?
            .take(0)
            .map_err(|e| {
                error!("Failed to deserialize coupling metrics: {}", e);
                CodeGraphError::Database(format!("Deserialization failed: {}", e))
            })?;

        results.into_iter().next().flatten().ok_or_else(|| {
            CodeGraphError::Database(format!(
                "Node not found or project_id mismatch: node_id='{}', expected project_id='{}'. \
                    Ensure the node exists and was indexed with the same project_id.",
                node_id, self.project_id
            ))
        })
    }

    /// Get hub nodes with degree >= min_degree
    ///
    /// # Arguments
    /// * `min_degree` - Minimum total degree (defaults to 5)
    ///
    /// # Returns
    /// Vector of highly connected hub nodes sorted by degree (descending)
    pub async fn get_hub_nodes(&self, min_degree: i32) -> Result<Vec<HubNode>> {
        debug!(
            "Calling fn::get_hub_nodes({}, project={})",
            min_degree, self.project_id
        );

        let result: Vec<HubNode> = self
            .db
            .query("RETURN fn::get_hub_nodes($project_id, $min_degree)")
            .bind(("project_id", self.project_id.clone()))
            .bind(("min_degree", min_degree))
            .await
            .map_err(|e| {
                error!("Failed to call get_hub_nodes: {}", e);
                CodeGraphError::Database(format!("get_hub_nodes failed: {}", e))
            })?
            .take(0)
            .map_err(|e| {
                error!("Failed to deserialize hub nodes: {}", e);
                CodeGraphError::Database(format!("Deserialization failed: {}", e))
            })?;

        Ok(result)
    }

    /// Get reverse dependencies (dependents) of a node
    ///
    /// # Arguments
    /// * `node_id` - The ID of the node to analyze
    /// * `edge_type` - The type of edge to follow
    /// * `depth` - Maximum depth to traverse (1-10, defaults to 3)
    ///
    /// # Returns
    /// Vector of nodes that depend on the target node
    pub async fn get_reverse_dependencies(
        &self,
        node_id: &str,
        edge_type: &str,
        depth: i32,
    ) -> Result<Vec<DependencyNode>> {
        debug!(
            "Calling fn::get_reverse_dependencies({}, {}, {}, project={})",
            node_id, edge_type, depth, self.project_id
        );

        let result: Vec<DependencyNode> = self
            .db
            .query("RETURN fn::get_reverse_dependencies($project_id, $node_id, $edge_type, $depth)")
            .bind(("project_id", self.project_id.clone()))
            .bind(("node_id", node_id.to_string()))
            .bind(("edge_type", edge_type.to_string()))
            .bind(("depth", depth))
            .await
            .map_err(|e| {
                error!("Failed to call get_reverse_dependencies: {}", e);
                CodeGraphError::Database(format!("get_reverse_dependencies failed: {}", e))
            })?
            .take(0)
            .map_err(|e| {
                error!("Failed to deserialize reverse dependencies: {}", e);
                CodeGraphError::Database(format!("Deserialization failed: {}", e))
            })?;

        Ok(result)
    }

    /// Get complexity hotspots - functions with high cyclomatic complexity and coupling
    ///
    /// # Arguments
    /// * `min_complexity` - Minimum cyclomatic complexity threshold (default: 5.0)
    /// * `limit` - Maximum number of results (1-100, default: 20)
    ///
    /// # Returns
    /// Vector of complexity hotspots sorted by risk_score (complexity × afferent_coupling)
    pub async fn get_complexity_hotspots(
        &self,
        min_complexity: f32,
        limit: i32,
    ) -> Result<Vec<ComplexityHotspot>> {
        debug!(
            "Calling fn::get_complexity_hotspots(min={}, limit={}, project={})",
            min_complexity, limit, self.project_id
        );

        let result: Vec<ComplexityHotspot> = self
            .db
            .query("RETURN fn::get_complexity_hotspots($project_id, $min_complexity, $limit)")
            .bind(("project_id", self.project_id.clone()))
            .bind(("min_complexity", min_complexity))
            .bind(("limit", limit))
            .await
            .map_err(|e| {
                error!("Failed to call get_complexity_hotspots: {}", e);
                CodeGraphError::Database(format!("get_complexity_hotspots failed: {}", e))
            })?
            .take(0)
            .map_err(|e| {
                error!("Failed to deserialize complexity hotspots: {}", e);
                CodeGraphError::Database(format!("Deserialization failed: {}", e))
            })?;

        Ok(result)
    }

    /// Count nodes for the current project (used for health checks)
    pub async fn count_nodes_for_project(&self) -> Result<usize> {
        let mut response = self
            .db
            .query("SELECT count() AS count FROM nodes WHERE project_id = $project_id GROUP ALL;")
            .bind(("project_id", self.project_id.clone()))
            .await
            .map_err(|e| {
                CodeGraphError::Database(format!("count_nodes_for_project query failed: {}", e))
            })?;

        let rows: Vec<serde_json::Value> = response.take(0).map_err(|e| {
            CodeGraphError::Database(format!("Failed to deserialize count rows: {}", e))
        })?;

        let count = rows
            .first()
            .and_then(|v| v.get("count"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        Ok(count as usize)
    }

    /// Find nodes by (partial) name within the current project
    pub async fn find_nodes_by_name(
        &self,
        needle: &str,
        limit: usize,
    ) -> Result<Vec<NodeReference>> {
        let max = limit.clamp(1, 50) as i64;

        debug!(
            "Calling fn::find_nodes_by_name({}, project={}, limit={})",
            needle, self.project_id, max
        );

        let result: Vec<NodeReference> = self
            .db
            .query("RETURN fn::find_nodes_by_name($project_id, $needle, $limit)")
            .bind(("project_id", self.project_id.clone()))
            .bind(("needle", needle.to_string()))
            .bind(("limit", max))
            .await
            .map_err(|e| {
                error!("Failed to call find_nodes_by_name: {}", e);
                CodeGraphError::Database(format!("find_nodes_by_name failed: {}", e))
            })?
            .take(0)
            .map_err(|e| {
                error!("Failed to deserialize find_nodes_by_name results: {}", e);
                CodeGraphError::Database(format!("Deserialization failed: {}", e))
            })?;

        Ok(result)
    }

    /// Comprehensive semantic search with HNSW vector search, full-text, and graph enrichment
    ///
    /// Calls fn::semantic_search_with_context in SurrealDB which combines:
    /// - HNSW vector similarity search
    /// - Full-text search using code_analyzer
    /// - Graph enrichment with dependencies and file context
    ///
    /// # Parameters
    /// - `query_text`: Original search query
    /// - `query_embedding`: Pre-generated embedding vector
    /// - `dimension`: Embedding dimension (384,768,1024,1536,2048,2560,3072,4096)
    /// - `limit`: Maximum results
    /// - `threshold`: Minimum similarity score (0.0-1.0)
    /// - `include_graph_context`: Whether to enrich with graph data
    pub async fn semantic_search_with_context(
        &self,
        query_text: &str,
        query_embedding: &[f32],
        dimension: usize,
        limit: usize,
        threshold: f32,
        _include_graph_context: bool,
    ) -> Result<Vec<serde_json::Value>> {
        // Always use node-level search (via chunks) to return enriched node records
        self.semantic_search_nodes_via_chunks(
            query_text,
            query_embedding,
            dimension,
            limit,
            threshold,
        )
        .await
    }

    #[allow(dead_code)]
    async fn semantic_search_chunks_with_context(
        &self,
        query_text: &str,
        query_embedding: &[f32],
        dimension: usize,
        limit: usize,
        threshold: f32,
        include_graph_context: bool,
    ) -> Result<Vec<serde_json::Value>> {
        debug!(
            "Calling fn::semantic_search_chunks_with_context(project={}, query='{}', dim={}, limit={}, threshold={})",
            self.project_id, query_text, dimension, limit, threshold
        );

        let embedding_value: serde_json::Value =
            serde_json::to_value(query_embedding).map_err(|e| {
                CodeGraphError::Database(format!("Failed to serialize embedding: {}", e))
            })?;

        let mut response = self
            .db
            .query("RETURN fn::semantic_search_chunks_with_context($project_id, $query_embedding, $query_text, $dimension, $limit, $threshold, $include_graph_context)")
            .bind(("project_id", self.project_id.clone()))
            .bind(("query_embedding", embedding_value))
            .bind(("query_text", query_text.to_string()))
            .bind(("dimension", dimension as i64))
            .bind(("limit", limit as i64))
            .bind(("threshold", threshold as f64))
            .bind(("include_graph_context", include_graph_context))
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("semantic_search_chunks_with_context") {
                    warn!(
                        "semantic_search_chunks_with_context missing in DB; falling back to fn::semantic_search_with_context"
                    );
                    CodeGraphError::Database("MISSING_CHUNK_FN".into())
                } else {
                    error!("Failed to call semantic_search_chunks_with_context: {}", msg);
                    CodeGraphError::Database(format!("semantic_search_chunks_with_context failed: {}", msg))
                }
            })?;

        // Workaround for SurrealDB 2.x SDK bug (GitHub #4921):
        // Direct deserialization to serde_json::Value fails with "invalid type: enum"
        // Instead, get raw SurrealDB Value and serialize via serde_json::to_value
        let raw_value: SurrealValue = response.take(0).map_err(|e| {
            error!(
                "Failed to get raw value from semantic_search_chunks_with_context: {}",
                e
            );
            CodeGraphError::Database(format!("Failed to get raw value: {}", e))
        })?;

        // Convert SurrealDB Value to clean JSON (avoids enum variant tags)
        let json_value = surreal_to_json(raw_value);

        // Extract Vec from the JSON value
        let result: Vec<serde_json::Value> = match json_value {
            serde_json::Value::Array(arr) => arr,
            serde_json::Value::Null => Vec::new(),
            other => vec![other],
        };

        Ok(result)
    }

    /// Semantic search that returns full node records with content, deduplicated from chunk matches.
    /// Uses chunk-level semantic search for precision, then deduplicates by parent node.
    /// Returns complete code units (functions, classes, etc.) with full content for context-engineering.
    pub async fn semantic_search_nodes_via_chunks(
        &self,
        query_text: &str,
        query_embedding: &[f32],
        dimension: usize,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<serde_json::Value>> {
        // Call the SurrealDB function directly
        let embedding_value = query_embedding.to_vec().into_value();
        let mut response = self
            .db
            .query("RETURN fn::semantic_search_nodes_via_chunks($project_id, $query_text, $dimension, $limit, $threshold, $query_embedding)")
            .bind(("project_id", self.project_id.clone()))
            .bind(("query_text", query_text.to_string()))
            .bind(("dimension", dimension as i64))
            .bind(("limit", limit as i64))
            .bind(("threshold", threshold as f64))
            .bind(("query_embedding", embedding_value))
            .await
            .map_err(|e| {
                error!("Failed to call semantic_search_nodes_via_chunks: {}", e);
                CodeGraphError::Database(format!("semantic_search_nodes_via_chunks failed: {}", e))
            })?;

        let raw_value: SurrealValue = response.take(0).map_err(|e| {
            error!(
                "Failed to get raw value from semantic_search_nodes_via_chunks: {}",
                e
            );
            CodeGraphError::Database(format!("Failed to get raw value: {}", e))
        })?;

        let json_value = surreal_to_json(raw_value);
        let result: Vec<serde_json::Value> = match json_value {
            serde_json::Value::Array(arr) => arr,
            serde_json::Value::Null => Vec::new(),
            other => vec![other],
        };

        Ok(result)
    }

    /// Get top directories by file count (lightweight summary)
    pub async fn get_top_directories(&self, limit: i32) -> Result<Vec<DirectorySummary>> {
        let safe_limit = limit.clamp(1, 50);

        let sql = r#"
            LET $dirs = (
              SELECT id, name, full_path, depth, metadata, project_id
              FROM nodes
              WHERE project_id = $project_id AND node_type = 'Directory'
              ORDER BY depth ASC, name ASC
              LIMIT 500
            );

            LET $counts = (
              SELECT dir.id AS dir_id, count() AS file_count
              FROM $dirs AS dir
              INNER JOIN edges ON edges.from = dir.id AND edges.edge_type = 'contains'
              INNER JOIN nodes AS n ON edges.to = n.id AND n.node_type != 'Directory'
              GROUP BY dir.id
              ORDER BY file_count DESC
              LIMIT $limit
            );

            RETURN (
              SELECT dir_id, file_count,
                     (SELECT VALUE name FROM $dirs WHERE id = dir_id)[0] AS name,
                     (SELECT VALUE full_path FROM $dirs WHERE id = dir_id)[0] AS full_path,
                     (SELECT VALUE depth FROM $dirs WHERE id = dir_id)[0] AS depth
              FROM $counts
            );
        "#;

        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", self.project_id.clone()))
            .bind(("limit", safe_limit))
            .await
            .map_err(|e| CodeGraphError::Database(format!("get_top_directories failed: {}", e)))?;

        let result: Vec<DirectorySummary> = response.take(0).map_err(|e| {
            CodeGraphError::Database(format!("Failed to deserialize directories: {}", e))
        })?;

        Ok(result)
    }
}

// ============================================================================
// Type Definitions for Function Results
// ============================================================================

/// Node with dependency depth information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyNode {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    pub location: Option<NodeLocation>,
    pub language: Option<String>,
    #[serde(
        default,
        deserialize_with = "codegraph_core::deserialize_content_string"
    )]
    pub content: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub dependency_depth: Option<i32>,
    pub dependent_depth: Option<i32>,
}

/// Circular dependency pair
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircularDependency {
    pub node1_id: String,
    pub node2_id: String,
    pub node1: NodeInfo,
    pub node2: NodeInfo,
    pub dependency_type: String,
}

/// Call chain node with caller information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallChainNode {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    pub location: Option<NodeLocation>,
    pub language: Option<String>,
    #[serde(
        default,
        deserialize_with = "codegraph_core::deserialize_content_string"
    )]
    pub content: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub call_depth: Option<i32>,
    pub called_by: Option<Vec<CallerInfo>>,
}

/// Coupling metrics result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouplingMetricsResult {
    pub node: NodeInfo,
    pub metrics: CouplingMetrics,
    pub dependents: Vec<NodeReference>,
    pub dependencies: Vec<NodeReference>,
}

/// Coupling metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouplingMetrics {
    pub afferent_coupling: i32,
    pub efferent_coupling: i32,
    pub total_coupling: i32,
    pub instability: f64,
    pub stability: f64,
    pub is_stable: bool,
    pub is_unstable: bool,
    pub coupling_category: String,
}

/// Hub node with degree information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubNode {
    pub node_id: String,
    pub node: NodeInfo,
    pub afferent_degree: i32,
    pub efferent_degree: i32,
    pub total_degree: i32,
    pub incoming_by_type: Vec<EdgeTypeCount>,
    pub outgoing_by_type: Vec<EdgeTypeCount>,
}

/// Directory summary for bootstrap
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorySummary {
    pub dir_id: String,
    pub name: String,
    pub full_path: String,
    pub depth: Option<i32>,
    pub file_count: i32,
}

/// Edge type count
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeTypeCount {
    pub edge_type: String,
    pub count: i32,
}

/// Node information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    pub location: Option<NodeLocation>,
    pub language: Option<String>,
    #[serde(
        default,
        deserialize_with = "codegraph_core::deserialize_content_string"
    )]
    pub content: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Node reference (minimal info)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeReference {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    pub location: Option<NodeLocation>,
}

/// Caller information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallerInfo {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
}

/// Node location information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLocation {
    pub file_path: String,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
}

/// Complexity hotspot with risk metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityHotspot {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    pub language: Option<String>,
    pub file_path: Option<String>,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
    pub complexity: f32,
    pub afferent_coupling: i32,
    pub efferent_coupling: i32,
    pub instability: f32,
    pub risk_score: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_sdk_values_preserve_content_defaults_record_ids_and_integer_precision() {
        let db: Surreal<Any> = Surreal::init();
        db.connect("mem://").await.unwrap();
        db.use_ns("test").use_db("conversion").await.unwrap();
        let content = "fn native_sdk() {}\n".repeat(100);
        let compressed = codegraph_core::compress_to_string(&content);
        let query = "RETURN [{ id: nodes:native, name: 'native_sdk', content: $content, metadata: { large: 9007199254740993 } }, { id: nodes:missing, name: 'missing_content' }];";

        let mut response = db
            .query(query)
            .bind(("content", compressed.clone()))
            .await
            .unwrap();
        let nodes: Vec<NodeInfo> = response.take(0).unwrap();
        assert_eq!(nodes[0].id, "nodes:native");
        assert_eq!(nodes[0].content.as_deref(), Some(content.as_str()));
        assert_eq!(
            nodes[0].metadata.as_ref().unwrap()["large"],
            json!(9007199254740993_i64)
        );
        assert!(nodes[1].content.is_none());

        let mut response = db.query(query).bind(("content", compressed)).await.unwrap();
        let raw: SurrealValue = response.take(0).unwrap();
        let json = surreal_to_json(raw);
        assert_eq!(json[0]["id"], "nodes:native");
        assert_eq!(json[0]["content"], content);
        assert_eq!(json[0]["metadata"]["large"], json!(9007199254740993_i64));
    }

    #[test]
    fn test_dependency_node_serialization() {
        let node = DependencyNode {
            id: "test:1".to_string(),
            name: "test_function".to_string(),
            kind: Some("function".to_string()),
            location: Some(NodeLocation {
                file_path: "test.rs".to_string(),
                start_line: Some(10),
                end_line: Some(20),
            }),
            language: Some("rust".to_string()),
            content: None,
            metadata: None,
            dependency_depth: Some(1),
            dependent_depth: None,
        };

        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("test_function"));
    }

    #[cfg(feature = "surrealdb")]
    #[tokio::test]
    async fn count_nodes_for_project_filters_by_project() {
        use surrealdb::opt::auth::Root;

        let db: Surreal<Any> = Surreal::init();
        db.connect("mem://").await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        db.signin(Root {
            username: "root".to_string(),
            password: "root".to_string(),
        })
        .await
        .ok(); // mem engine ignores auth

        // Two projects, only one should be counted
        db.query("CREATE nodes CONTENT $doc")
            .bind((
                "doc",
                json!({
                    "id": "nodes:a1",
                    "name": "A1",
                    "project_id": "proj-a"
                }),
            ))
            .await
            .unwrap();

        db.query("CREATE nodes CONTENT $doc")
            .bind((
                "doc",
                json!({
                    "id": "nodes:b1",
                    "name": "B1",
                    "project_id": "proj-b"
                }),
            ))
            .await
            .unwrap();

        let gf = GraphFunctions::new_with_project_id(Arc::new(db), "proj-a");
        let count = gf.count_nodes_for_project().await.unwrap();

        assert_eq!(count, 1, "Should only count nodes in proj-a");
    }
}

crate::impl_surreal_serde!(
    DependencyNode,
    CircularDependency,
    CallChainNode,
    CouplingMetricsResult,
    CouplingMetrics,
    HubNode,
    DirectorySummary,
    EdgeTypeCount,
    NodeInfo,
    NodeReference,
    CallerInfo,
    NodeLocation,
    ComplexityHotspot
);
