// ABOUTME: Independently migrated SurrealKV memory persistence and scoped search indexes.
// ABOUTME: The sole owner serializes transitions and commits source, derived data, and jobs atomically.
use crate::records::{self, Rows};
use crate::types::*;
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;
use surrealdb::types::Value;
use surrealdb::{Surreal, engine::any::Any};

pub fn digest<T: serde::Serialize>(value: &T) -> Result<String> {
    Ok(Sha256::digest(serde_json::to_vec(value)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
pub fn content_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub struct Store {
    pub state: State,
    db: Surreal<Any>,
    rows: Rows,
    projection_ready: bool,
}
impl Store {
    pub async fn open(path: Option<&Path>) -> Result<Self> {
        let db: Surreal<Any> = Surreal::init();
        let connection = match path {
            Some(path) => {
                ensure!(
                    !path.exists() || !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
                    "Memory database cannot be a symlink"
                );
                std::fs::create_dir_all(path.parent().context("memory store needs a parent")?)?;
                format!("surrealkv://{}", path.display())
            }
            None => "mem://".into(),
        };
        db.connect(connection).await?;
        #[cfg(unix)]
        if let Some(path) = path {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
                "Memory database cannot be a symlink"
            );
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
        }
        // Embedded stores share library-level index caches. Stable unique namespaces prevent
        // collisions across project/user stores while surviving repository moves.
        let store_id = if let Some(path) = path {
            let identity_path = path.with_extension("identity");
            if identity_path.exists() {
                std::fs::read_to_string(identity_path)?
            } else {
                use std::io::Write;
                let id = uuid::Uuid::new_v4().simple().to_string();
                let mut file = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(identity_path)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                }
                file.write_all(id.as_bytes())?;
                file.sync_all()?;
                id
            }
        } else {
            uuid::Uuid::new_v4().simple().to_string()
        };
        ensure!(
            store_id.len() == 32 && store_id.bytes().all(|v| v.is_ascii_hexdigit()),
            "Invalid memory store identity"
        );
        db.use_ns(format!("codegraph_memory_{store_id}"))
            .use_db("v1")
            .await?;
        Self::initialize(db).await
    }

    async fn initialize(db: Surreal<Any>) -> Result<Self> {
        db.query(include_str!("../../../schema/agent_memory_queries.surql"))
            .await?
            .check()?;
        let info: Value = db.query("INFO FOR DB").await?.check()?.take(0)?;
        let info = records::serde_json_value(info)?;
        let has_table = |table: &str| info["tables"].get(table).is_some();
        let meta: Vec<Value> = if has_table("memory_meta") {
            db.query("SELECT * FROM memory_meta:main")
                .await?
                .check()?
                .take(0)?
        } else {
            Vec::new()
        };
        if !meta.is_empty() {
            db.query(include_str!("../../../schema/agent_memory_v2.surql"))
                .await?
                .check()?;
            let projection_ready =
                records::serde_json_value(meta[0].clone())?["projection_version"] == 1;
            let state = Self::load(&db).await?;
            let rows = records::rows(&state)?;
            let mut store = Self {
                state,
                db,
                rows,
                projection_ready,
            };
            if !projection_ready {
                store.commit(store.state.clone()).await?;
            }
            return Ok(store);
        }
        // v1 is an immutable migration source until the v2 transaction succeeds.
        let payloads: Vec<String> = if has_table("memory_state") {
            db.query("SELECT VALUE payload FROM memory_state:main")
                .await?
                .check()?
                .take(0)?
        } else {
            Vec::new()
        };
        let mut legacy: State = match payloads.first() {
            Some(payload) => serde_json::from_str(payload)?,
            None => State {
                version: 2,
                ..State::default()
            },
        };
        ensure!(
            matches!(legacy.version, 1 | 2),
            "Unsupported memory schema version {}",
            legacy.version
        );
        legacy.version = 2;
        let mut store = Self {
            state: State {
                version: 2,
                ..State::default()
            },
            db,
            rows: Rows::new(),
            projection_ready: false,
        };
        store.persist(legacy, true).await?;
        Ok(store)
    }

    /// Native records are authoritative; the policy state is a reloadable owner-local cache.
    async fn load(db: &Surreal<Any>) -> Result<State> {
        let mut response = db.query("SELECT * FROM memory_meta:main; SELECT * FROM memory_claim; SELECT * FROM memory_observation; SELECT * FROM memory_revision; SELECT * FROM memory_job; SELECT * FROM memory_relationship; SELECT * FROM memory_idempotency; SELECT * FROM memory_tombstone;").await?.check()?;
        let meta: Vec<Value> = response.take(0)?;
        let meta =
            records::serde_json_value(meta.into_iter().next().context("Memory metadata missing")?)?;
        ensure!(
            meta["schema_version"] == 2,
            "Unsupported memory schema version"
        );
        let mut state = State {
            version: 2,
            generation: meta["generation"]
                .as_u64()
                .context("Invalid memory generation")?,
            embedding_identity: meta["embedding_identity"].as_str().map(str::to_string),
            ..State::default()
        };
        for (index, identity) in [
            (1, "memory_id"),
            (2, "observation_id"),
            (3, ""),
            (4, "operation_id"),
            (5, ""),
            (6, ""),
            (7, ""),
        ] {
            let rows: Vec<Value> = response.take(index)?;
            for row in rows {
                let mut row = records::serde_json_value(row)?;
                row.as_object_mut()
                    .context("Memory record is not an object")?
                    .remove("id");
                if !identity.is_empty() {
                    row["id"] = row[identity].clone();
                }
                match index {
                    1 => {
                        let claim: Claim = serde_json::from_value(row)?;
                        state.claims.insert(claim.id.clone(), claim);
                    }
                    2 => {
                        let observation: Observation = serde_json::from_value(row)?;
                        state
                            .observations
                            .insert(observation.id.clone(), observation);
                    }
                    3 => state.revisions.push(serde_json::from_value(row)?),
                    4 => {
                        let operation: Operation = serde_json::from_value(row)?;
                        state.operations.insert(operation.id.clone(), operation);
                    }
                    5 => state.relationships.push(serde_json::from_value(row)?),
                    6 => {
                        state.idempotency.insert(
                            row["digest"]
                                .as_str()
                                .context("Invalid idempotency digest")?
                                .into(),
                            (
                                row["content_digest"]
                                    .as_str()
                                    .context("Invalid content digest")?
                                    .into(),
                                row["operation_id"]
                                    .as_str()
                                    .context("Invalid operation ID")?
                                    .into(),
                            ),
                        );
                    }
                    7 => {
                        state.tombstones.insert(
                            row["memory_id"]
                                .as_str()
                                .context("Invalid tombstone")?
                                .into(),
                            row["generation"]
                                .as_u64()
                                .context("Invalid tombstone generation")?,
                        );
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(state)
    }
    pub fn db(&self) -> &Surreal<Any> {
        &self.db
    }

    /// Commit only changed records/vectors under one generation-checked transaction.
    pub async fn commit(&mut self, next: State) -> Result<()> {
        self.persist(next, false).await
    }
    async fn persist(&mut self, mut next: State, migrate: bool) -> Result<()> {
        next.version = 2;
        let generation = if migrate {
            next.generation
        } else {
            self.state.generation
        };
        ensure!(
            generation < i64::MAX as u64,
            "Memory generation exceeds SurrealDB's signed range"
        );
        next.generation = generation + 1;
        if let Some(identity) = &next.embedding_identity {
            let mut dimension = None;
            for claim in next
                .claims
                .values()
                .filter(|claim| !claim.vectors.is_empty())
            {
                ensure!(
                    claim.embedding_identity == *identity,
                    "Incompatible embedding identity"
                );
                for vector in &claim.vectors {
                    ensure!(!vector.is_empty(), "Empty memory vector");
                    ensure!(
                        dimension.is_none_or(|d| d == vector.len()),
                        "Incompatible dimensions in memory store"
                    );
                    dimension = Some(vector.len());
                }
            }
            if let Some(dimension) = dimension {
                let previous_dimension = self
                    .state
                    .claims
                    .values()
                    .find(|claim| {
                        claim.embedding_identity == *identity && !claim.vectors.is_empty()
                    })
                    .map(|claim| claim.vectors[0].len());
                ensure!(
                    previous_dimension.is_none_or(|previous| previous == dimension),
                    "Dimension changed without a new embedding identity"
                );
                if migrate || previous_dimension.is_none() {
                    let schema = include_str!("../../../schema/agent_memory_vectors.surql")
                        .replace("__TABLE__", &vector_table(identity)?)
                        .replace("__DIMENSION__", &dimension.to_string());
                    self.db.query(schema).await?.check()?;
                }
            }
        }
        let projection_changes = projection_changes(&self.state, &next, !self.projection_ready)?;
        let rows = records::rows(&next)?;
        let mut updates = Vec::new();
        let mut edges = Vec::new();
        for (key, row) in &rows {
            if self.rows.get(key) != Some(row) {
                if row.endpoints.is_some() {
                    edges.push(row.binding());
                } else {
                    updates.push(row.binding());
                }
            }
        }
        let deletes: Vec<_> = self
            .rows
            .iter()
            .filter(|(key, _)| !rows.contains_key(*key))
            .map(|(_, row)| row.record.clone())
            .collect();
        let mut query = String::from("BEGIN TRANSACTION;");
        if migrate {
            query.push_str("REMOVE TABLE IF EXISTS memory_claim; REMOVE TABLE IF EXISTS memory_observation; REMOVE TABLE IF EXISTS memory_revision; REMOVE TABLE IF EXISTS memory_relationship; REMOVE TABLE IF EXISTS memory_job; REMOVE TABLE IF EXISTS memory_tombstone;");
            query.push_str(include_str!("../../../schema/agent_memory_v2.surql"));
        } else {
            query.push_str("LET $current = (SELECT VALUE generation FROM ONLY memory_meta:main) ?? 0; IF $current != $generation { THROW 'Memory generation conflict'; };");
        }
        query.push_str("FOR $id IN $deletes { DELETE $id; }; FOR $row IN $updates { UPSERT $row.id CONTENT $row.content; }; FOR $row IN $edges { LET $from=$row.in; LET $edge=$row.id; LET $to=$row.out; RELATE $from->$edge->$to CONTENT $row.content; }; FOR $change IN $projection_changes { LET $id=type::record('memory_projection_change',$change.memory_id); UPSERT $id CONTENT $change; }; UPSERT memory_meta:main CONTENT $meta;");
        if migrate {
            query.push_str("REMOVE TABLE IF EXISTS memory_state;");
            if let Some(identity) = &next.embedding_identity {
                query.push_str(&format!(
                    "REMOVE TABLE IF EXISTS memory_vector_{};",
                    digest(identity)?
                ));
            }
        }
        query.push_str("COMMIT TRANSACTION;");
        let mut meta =
            json!({"schema_version":2,"generation":next.generation,"projection_version":1});
        if let Some(identity) = &next.embedding_identity {
            meta["embedding_identity"] = identity.clone().into();
        }
        self.db
            .query(query)
            .bind(("generation", self.state.generation))
            .bind(("updates", updates))
            .bind(("edges", edges))
            .bind(("deletes", deletes))
            .bind(("meta", meta))
            .bind(("projection_changes", projection_changes))
            .await?
            .check()?;
        self.state = next;
        self.rows = rows;
        self.projection_ready = true;
        Ok(())
    }

    /// Bounded project-only deltas. The code projection never receives memory text or user/session content.
    pub async fn projection_changes(
        &self,
        context: &ClientContext,
        graph_project_id: &str,
        cursor: codegraph_core::memory_projection::ProjectionCursor,
    ) -> Result<codegraph_core::memory_projection::ProjectionBatch> {
        use codegraph_core::memory_projection::*;
        let mut rows:Vec<serde_json::Value>=self.db.query(
            "SELECT memory_id,revision,generation,enabled,anchors FROM memory_projection_change WHERE owner_id=$owner AND project_id=$project AND (generation>$generation OR (generation=$generation AND memory_id>$memory_id)) ORDER BY generation ASC,memory_id ASC LIMIT 257;"
        ).bind(("owner",context.owner_id.clone())).bind(("project",context.project_id.clone()))
            .bind(("generation",cursor.generation)).bind(("memory_id",cursor.memory_id)).await?.check()?.take(0)?;
        let complete = rows.len() <= 256;
        rows.truncate(256);
        let mut changes: Vec<ProjectionChange> = rows
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?;
        for change in &mut changes {
            change
                .anchors
                .retain(|anchor| anchor.graph_project_id == graph_project_id);
        }
        let cursor = if complete {
            ProjectionCursor {
                generation: self.state.generation,
                memory_id: "\u{10ffff}".into(),
            }
        } else {
            let last = changes.last().context("Missing projection cursor")?;
            ProjectionCursor {
                generation: last.generation,
                memory_id: last.memory_id.clone(),
            }
        };
        let batch = ProjectionBatch {
            source_id: digest(&(&context.owner_id, &context.project_id, graph_project_id))?,
            cursor,
            complete,
            changes,
        };
        ensure!(
            serde_json::to_vec(&batch)?.len() <= 6 * 1024 * 1024,
            "Memory projection exceeds the bounded IPC allowance"
        );
        Ok(batch)
    }

    pub async fn hybrid_candidates(
        &self,
        identity: &str,
        query: &str,
        vector: Vec<f32>,
        eligible: Vec<String>,
        node_ids: Vec<String>,
        graph_candidates: Vec<String>,
    ) -> Result<Vec<(String, f64)>> {
        if eligible.is_empty() {
            return Ok(Vec::new());
        }
        ensure!(
            self.state.embedding_identity.as_deref() == Some(identity),
            "Memory embedding identity is incompatible; re-embedding is required"
        );
        let table = vector_table(identity)?;
        let mut response=self.db.query(format!(
            "LET $chunks = SELECT memory_id, vector::distance::knn() AS distance FROM {table} WHERE memory_id INSIDE $eligible AND vector <|400,800|> $vector;
             LET $semantic = SELECT memory_id AS id, math::min(distance) AS distance FROM $chunks GROUP BY memory_id ORDER BY distance ASC,id ASC LIMIT 100;
             LET $lexical = SELECT memory_id AS id, search::score(1) AS score FROM memory_claim WHERE memory_id INSIDE $eligible AND statement @1@ $query ORDER BY score DESC,id ASC LIMIT 100;
             LET $linked = SELECT in.memory_id AS memory_id FROM memory_link WHERE in.memory_id INSIDE $eligible AND revision=in.revision AND out.node_id INSIDE $node_ids;
             LET $graph = SELECT memory_id AS id FROM memory_claim WHERE memory_id INSIDE $eligible AND (memory_id INSIDE $linked.memory_id OR memory_id INSIDE $graph_candidates) ORDER BY id ASC LIMIT 100;
             RETURN fn::memory_fuse([$semantic,$lexical,$graph]);"
        )).bind(("eligible",eligible)).bind(("vector",vector)).bind(("query",query.to_string()))
            .bind(("node_ids",node_ids)).bind(("graph_candidates",graph_candidates)).await?.check()?;
        let index = response.num_statements() - 1;
        let rows: Vec<serde_json::Value> = response.take(index)?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    row["id"]
                        .as_str()
                        .context("Missing fused memory ID")?
                        .into(),
                    row["score"].as_f64().context("Missing fused score")?,
                ))
            })
            .collect()
    }
    pub async fn fuse_lists(&self, lists: Vec<Vec<String>>) -> Result<Vec<(String, f64)>> {
        ensure!(
            lists.len() <= 4 && lists.iter().all(|list| list.len() <= 100),
            "Unbounded fusion candidates"
        );
        let lists: Vec<_> = lists
            .into_iter()
            .map(|list| {
                list.into_iter()
                    .map(|id| json!({"id":id}))
                    .collect::<Vec<_>>()
            })
            .collect();
        let rows: Vec<serde_json::Value> = self
            .db
            .query("RETURN fn::memory_fuse($lists)")
            .bind(("lists", lists))
            .await?
            .check()?
            .take(0)?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    row["id"]
                        .as_str()
                        .context("Missing fused memory ID")?
                        .into(),
                    row["score"].as_f64().context("Missing fused score")?,
                ))
            })
            .collect()
    }
}
pub(crate) fn vector_table(identity: &str) -> Result<String> {
    Ok(format!("memory_embedding_{}", digest(&identity)?))
}

fn projection_changes(
    previous: &State,
    next: &State,
    seed: bool,
) -> Result<Vec<serde_json::Value>> {
    use codegraph_core::memory_projection::ProjectionAnchor;
    fn content(claim: &Claim) -> serde_json::Value {
        let enabled = !matches!(
            claim.lifecycle,
            Lifecycle::Provisional | Lifecycle::Retracted
        );
        let anchors: Vec<_> = if enabled {
            claim
                .anchors
                .iter()
                .map(|anchor| ProjectionAnchor {
                    graph_project_id: anchor.graph_project_id.clone(),
                    node_id: anchor.node_id.clone(),
                    supporting: anchor.supporting,
                })
                .collect()
        } else {
            Vec::new()
        };
        json!({"memory_id":claim.id,"owner_id":claim.owner_id,"project_id":claim.project_id,"revision":claim.revision,"enabled":enabled,"anchors":anchors})
    }
    let mut changes = Vec::new();
    for claim in next
        .claims
        .values()
        .filter(|claim| claim.scope == Scope::Project)
    {
        let mut value = content(claim);
        if seed
            || previous
                .claims
                .get(&claim.id)
                .is_none_or(|old| old.scope != Scope::Project || content(old) != value)
        {
            value["generation"] = next.generation.into();
            changes.push(value);
        }
    }
    for old in previous
        .claims
        .values()
        .filter(|claim| claim.scope == Scope::Project)
    {
        if next
            .claims
            .get(&old.id)
            .is_none_or(|claim| claim.scope != Scope::Project)
        {
            changes.push(json!({"memory_id":old.id,"owner_id":old.owner_id,"project_id":old.project_id,"revision":old.revision+1,"generation":next.generation,"enabled":false,"anchors":[]}));
        }
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    fn fixture() -> State {
        let now = Utc::now();
        let claim = Claim {
            id: "claim-a".into(),
            revision: 1,
            statement: "Preserve scalar scoring".into(),
            owner_id: "alice".into(),
            project_id: "p".into(),
            session_id: None,
            scope: Scope::Project,
            kind: Kind::Decision,
            tier: Tier::Durable,
            lifecycle: Lifecycle::Active,
            evidence_state: EvidenceState::Reported,
            grounding: Grounding::Anchored,
            evidence: vec![Evidence {
                uri: "test:scoring".into(),
                ..Default::default()
            }],
            anchors: vec![Anchor {
                node_id: "nodes:score".into(),
                graph_project_id: "p".into(),
                supporting: true,
                ..Default::default()
            }],
            observation_ids: vec!["obs-a".into()],
            applicability: Default::default(),
            recorded_at: now,
            observed_at: now,
            valid_from: now,
            valid_until: None,
            expires_at: None,
            review_after: None,
            confirmed_at: None,
            review_reasons: vec![],
            conflicts: vec![],
            classification_identity: Some("classifier-v1".into()),
            embedding_identity: "mock-v1".into(),
            vectors: vec![vec![1.0, 0.0, 0.0]],
            usefulness: 0,
            feedback_ids: vec![],
        };
        let observation = Observation {
            id: "obs-a".into(),
            context: ClientContext {
                owner_id: "alice".into(),
                project_id: "p".into(),
                session_id: None,
                task_id: None,
                context_epoch: None,
                applicability: Default::default(),
            },
            request: serde_json::from_value(json!({"statement":"Preserve scalar scoring"}))
                .unwrap(),
            recorded_at: now,
        };
        State {
            version: 1,
            generation: 7,
            claims: std::collections::BTreeMap::from([(claim.id.clone(), claim)]),
            observations: std::collections::BTreeMap::from([(observation.id.clone(), observation)]),
            embedding_identity: Some("mock-v1".into()),
            idempotency: std::collections::BTreeMap::from([(
                "retry-key".into(),
                ("digest".into(), "op-a".into()),
            )]),
            ..State::default()
        }
    }
    async fn legacy(state: &State) -> Surreal<Any> {
        let db: Surreal<Any> = Surreal::init();
        db.connect("mem://").await.unwrap();
        db.use_ns(uuid::Uuid::new_v4().simple().to_string())
            .use_db("memory")
            .await
            .unwrap();
        db.query(include_str!("../../../schema/agent_memory_v1.surql"))
            .await
            .unwrap()
            .check()
            .unwrap();
        db.query("CREATE memory_state:main SET payload=$payload;")
            .bind(("payload", serde_json::to_string(state).unwrap()))
            .await
            .unwrap()
            .check()
            .unwrap();
        db
    }
    #[tokio::test]
    async fn migration_preserves_typed_records_and_native_derivation_paths() {
        let original = fixture();
        let db = legacy(&original).await;
        let store = Store::initialize(db.clone()).await.unwrap();
        assert_eq!(store.state.version, 2);
        assert_eq!(
            store.state.claims["claim-a"].statement,
            original.claims["claim-a"].statement
        );
        assert_eq!(store.state.idempotency, original.idempotency);
        let loaded = Store::initialize(db.clone()).await.unwrap();
        assert_eq!(
            loaded.state.claims["claim-a"].vectors,
            vec![vec![1.0, 0.0, 0.0]]
        );
        let mut response=db.query("SELECT ->memory_link->memory_code_locator.node_id AS nodes, ->memory_derived_from->memory_observation.request.statement AS sources FROM memory_claim; INFO FOR DB;").await.unwrap().check().unwrap();
        let paths: Vec<serde_json::Value> = response.take(0).unwrap();
        assert_eq!(paths[0]["nodes"][0], "nodes:score");
        assert_eq!(paths[0]["sources"][0], "Preserve scalar scoring");
        let info: Value = response.take(1).unwrap();
        let info = records::serde_json_value(info).unwrap();
        assert!(info["tables"].get("memory_state").is_none());
        let invalid = db
            .query("UPDATE memory_claim SET scope='organization';")
            .await
            .unwrap()
            .check();
        assert!(invalid.is_err());
    }
    #[tokio::test]
    async fn failed_migration_retains_legacy_payload() {
        let mut original = fixture();
        original.claims.get_mut("claim-a").unwrap().scope = Scope::Session;
        // Invalid stored type cannot be silently accepted by the v2 schema.
        let db = legacy(&original).await;
        let mut payload = serde_json::to_value(&original).unwrap();
        payload["claims"]["claim-a"]["revision"] = json!(u64::MAX);
        db.query("UPDATE memory_state:main SET payload=$payload;")
            .bind(("payload", payload.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
        assert!(Store::initialize(db.clone()).await.is_err());
        let rows: Vec<String> = db
            .query("SELECT VALUE payload FROM memory_state:main")
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(rows[0], payload.to_string());
        let mut overflowing = fixture();
        overflowing.generation = u64::MAX;
        let db = legacy(&overflowing).await;
        assert!(Store::initialize(db.clone()).await.is_err());
        let rows: Vec<String> = db
            .query("SELECT VALUE payload FROM memory_state:main")
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(rows[0], serde_json::to_string(&overflowing).unwrap());
    }
    #[tokio::test]
    async fn unrelated_commits_preserve_vector_rows_and_generation_conflicts_roll_back() {
        let mut store = Store::initialize(legacy(&fixture()).await).await.unwrap();
        let table = vector_table("mock-v1").unwrap();
        store.db.query(format!("DEFINE FIELD marker ON {table} TYPE option<string>; UPDATE {table} SET marker='untouched';")).await.unwrap().check().unwrap();
        let mut next = store.state.clone();
        next.claims.get_mut("claim-a").unwrap().usefulness = 1;
        store.commit(next).await.unwrap();
        let markers: Vec<String> = store
            .db
            .query(format!("SELECT VALUE marker FROM {table}"))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(markers, vec!["untouched"]);
        let mut next = store.state.clone();
        next.claims.get_mut("claim-a").unwrap().statement = "Changed".into();
        store
            .db
            .query("UPDATE memory_meta:main SET generation+=1;")
            .await
            .unwrap()
            .check()
            .unwrap();
        assert!(store.commit(next).await.is_err());
        assert_eq!(
            store.state.claims["claim-a"].statement,
            "Preserve scalar scoring"
        );
        let statements: Vec<String> = store
            .db
            .query("SELECT VALUE statement FROM memory_claim")
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(statements, vec!["Preserve scalar scoring"]);
    }

    #[tokio::test]
    async fn native_fusion_collapses_chunks_and_uses_hnsw_bm25_and_locator_edges() {
        let mut store = Store::initialize(legacy(&fixture()).await).await.unwrap();
        let mut next = store.state.clone();
        next.claims.get_mut("claim-a").unwrap().vectors = vec![vec![1.0, 0.0, 0.0]; 3];
        let mut other = next.claims["claim-a"].clone();
        other.id = "claim-b".into();
        other.statement = "Cats prefer naps".into();
        other.anchors.clear();
        other.vectors = vec![vec![0.0, 1.0, 0.0]];
        next.claims.insert(other.id.clone(), other);
        store.commit(next).await.unwrap();
        let ranked = store
            .hybrid_candidates(
                "mock-v1",
                "missing",
                vec![1.0, 0.0, 0.0],
                vec!["claim-a".into()],
                vec![],
                vec![],
            )
            .await
            .unwrap();
        assert_eq!(ranked.len(), 1);
        assert!(
            (ranked[0].1 - 1.0 / 61.0).abs() < 1e-12,
            "Chunk duplicates must not accumulate RRF votes"
        );
        let ranked = store
            .hybrid_candidates(
                "mock-v1",
                "missing",
                vec![0.0, 1.0, 0.0],
                vec!["claim-a".into(), "claim-b".into()],
                vec!["nodes:score".into()],
                vec![],
            )
            .await
            .unwrap();
        assert_eq!(
            ranked[0].0, "claim-a",
            "The native anchor stream contributes to fusion"
        );
        let table = vector_table("mock-v1").unwrap();
        let vector_plan: Value = store
            .db
            .query(format!(
                "SELECT memory_id FROM {table} WHERE vector <|10,100|> [1.0,0.0,0.0] EXPLAIN;"
            ))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert!(format!("{vector_plan:?}").contains("memory_vector_hnsw"));
        let text_plan: Value = store
            .db
            .query("SELECT memory_id FROM memory_claim WHERE statement @1@ 'scoring' EXPLAIN;")
            .await
            .unwrap()
            .check()
            .unwrap()
            .take(0)
            .unwrap();
        assert!(format!("{text_plan:?}").contains("memory_claim_text_v2"));
    }

    #[tokio::test]
    async fn native_fusion_has_deterministic_ties_and_ignores_empty_streams() {
        let store = Store::open(None).await.unwrap();
        let a = (0..100)
            .map(|index| format!("a{index:03}"))
            .collect::<Vec<_>>();
        let b = (0..100)
            .map(|index| format!("b{index:03}"))
            .collect::<Vec<_>>();
        let ranked = store.fuse_lists(vec![a, b, vec![]]).await.unwrap();
        assert_eq!(ranked.len(), 100);
        assert_eq!(ranked[0].0, "a000");
        assert_eq!(ranked[1].0, "b000");
        assert_eq!(ranked[98].0, "a049");
        assert_eq!(ranked[99].0, "b049");
        assert!((ranked[0].1 - 1.0 / 122.0).abs() < 1e-12);
        assert!(
            store
                .fuse_lists(vec![vec![], vec![]])
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn projection_deltas_paginate_same_generation_and_exclude_user_session_content() {
        use codegraph_core::memory_projection::ProjectionCursor;
        let mut store = Store::open(None).await.unwrap();
        let mut state = fixture();
        let template = state.claims["claim-a"].clone();
        state.claims.clear();
        for index in 0..260 {
            let mut claim = template.clone();
            claim.id = format!("claim-{index:03}");
            state.claims.insert(claim.id.clone(), claim);
        }
        for (id, scope) in [("user", Scope::User), ("session", Scope::Session)] {
            let mut claim = template.clone();
            claim.id = id.into();
            claim.scope = scope;
            claim.session_id = Some("task".into());
            state.claims.insert(claim.id.clone(), claim);
        }
        store.commit(state).await.unwrap();
        let context = ClientContext {
            owner_id: "alice".into(),
            project_id: "p".into(),
            session_id: None,
            task_id: None,
            context_epoch: None,
            applicability: Default::default(),
        };
        let first = store
            .projection_changes(&context, "p", ProjectionCursor::default())
            .await
            .unwrap();
        assert_eq!(first.changes.len(), 256);
        assert!(!first.complete);
        let second = store
            .projection_changes(&context, "p", first.cursor)
            .await
            .unwrap();
        assert_eq!(second.changes.len(), 4);
        assert!(second.complete);
        let empty = store
            .projection_changes(&context, "p", second.cursor.clone())
            .await
            .unwrap();
        assert!(empty.changes.is_empty());
        let mut next = store.state.clone();
        next.claims.remove("claim-000");
        store.commit(next).await.unwrap();
        let deletion = store
            .projection_changes(&context, "p", second.cursor)
            .await
            .unwrap();
        assert_eq!(deletion.changes.len(), 1);
        assert!(!deletion.changes[0].enabled);
        assert!(deletion.changes[0].anchors.is_empty());
        let mut foreign = context.clone();
        foreign.owner_id = "bob".into();
        assert!(
            store
                .projection_changes(&foreign, "p", ProjectionCursor::default())
                .await
                .unwrap()
                .changes
                .is_empty()
        );
    }
}
