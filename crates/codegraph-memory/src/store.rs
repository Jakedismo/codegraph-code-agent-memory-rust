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
            let state = Self::load(&db).await?;
            let rows = records::rows(&state)?;
            return Ok(Self { state, db, rows });
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
        next.generation = if migrate {
            next.generation + 1
        } else {
            self.state.generation + 1
        };
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
        query.push_str("FOR $id IN $deletes { DELETE $id; }; FOR $row IN $updates { UPSERT $row.id CONTENT $row.content; }; FOR $row IN $edges { LET $from=$row.in; LET $edge=$row.id; LET $to=$row.out; RELATE $from->$edge->$to CONTENT $row.content; }; UPSERT memory_meta:main CONTENT $meta;");
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
        let mut meta = json!({"schema_version":2,"generation":next.generation});
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
            .await?
            .check()?;
        self.state = next;
        self.rows = rows;
        Ok(())
    }

    pub async fn candidates(
        &self,
        identity: &str,
        query: &str,
        vector: Vec<f32>,
        eligible: Vec<String>,
    ) -> Result<(Vec<String>, Vec<String>)> {
        if eligible.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        ensure!(
            self.state.embedding_identity.as_deref() == Some(identity),
            "Memory embedding identity is incompatible; re-embedding is required"
        );
        let table = vector_table(identity)?;
        let mut results = self.db.query(format!(
            "SELECT memory_id, vector::distance::knn() AS distance FROM {table} WHERE memory_id INSIDE $eligible AND vector <|100,200|> $vector ORDER BY distance; SELECT memory_id, search::score(1) AS score FROM memory_claim WHERE memory_id INSIDE $eligible AND statement @1@ $query ORDER BY score DESC LIMIT 100;"
        )).bind(("eligible",eligible)).bind(("vector",vector)).bind(("query",query.to_string())).await?.check()?;
        let semantic: Vec<serde_json::Value> = results.take(0)?;
        let lexical: Vec<serde_json::Value> = results.take(1)?;
        let ids = |rows: Vec<serde_json::Value>| {
            let mut ids = Vec::new();
            for row in rows {
                if let Some(id) = row["memory_id"].as_str()
                    && !ids.iter().any(|v| v == id)
                {
                    ids.push(id.to_string());
                }
            }
            ids
        };
        Ok((ids(semantic), ids(lexical)))
    }
}
pub(crate) fn vector_table(identity: &str) -> Result<String> {
    Ok(format!("memory_embedding_{}", digest(&identity)?))
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
}
