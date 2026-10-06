// ABOUTME: Independently migrated SurrealKV memory persistence and scoped search indexes.
// ABOUTME: The sole owner serializes transitions and commits source, derived data, and jobs atomically.
use crate::types::*;
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;
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
        db.query(include_str!("../../../schema/agent_memory_v1.surql"))
            .await?
            .check()?;
        let payloads: Vec<String> = db
            .query("SELECT VALUE payload FROM memory_state:main")
            .await?
            .check()?
            .take(0)?;
        let state: State = match payloads.first() {
            Some(payload) => serde_json::from_str(payload)?,
            None => State {
                version: 1,
                ..State::default()
            },
        };
        ensure!(
            state.version == 1,
            "Unsupported memory schema version {}",
            state.version
        );
        Ok(Self { state, db })
    }

    /// Persist before publishing an in-memory transition. Caller holds the owner mutex.
    pub async fn commit(&mut self, mut next: State) -> Result<()> {
        next.generation = self.state.generation + 1;
        let payload = serde_json::to_string(&next)?;
        let mut query = String::from(
            "BEGIN TRANSACTION; UPSERT memory_state:main SET payload = $payload; DELETE memory_claim; DELETE memory_observation; DELETE memory_revision; DELETE memory_relationship; DELETE memory_job; DELETE memory_tombstone;",
        );
        let claims: Vec<_> = next
            .claims
            .values()
            .map(|claim| {
                json!({
                    "memory_id": claim.id, "statement": claim.statement,
                    "owner_id": claim.owner_id, "project_id": claim.project_id,
                    "scope": claim.scope, "session_id": claim.session_id,
                })
            })
            .collect();
        query.push_str(" INSERT INTO memory_claim $claims;");
        // Strings keep the typed history independent of SurrealDB's object/date coercions.
        let observations: Vec<_> = next.observations.values().map(|v| json!({"key":v.id,"payload":serde_json::to_string(v).expect("serializable observation")})).collect();
        let revisions: Vec<_> = next.revisions.iter().map(|v| json!({"key":v.claim.id,"payload":serde_json::to_string(v).expect("serializable revision")})).collect();
        let relationships: Vec<_> = next
            .relationships
            .iter()
            .map(|v| json!({"source":v.source,"target":v.target,"kind":v.kind}))
            .collect();
        let jobs: Vec<_> = next.operations.values().map(|v| json!({"key":v.id,"payload":serde_json::to_string(v).expect("serializable operation")})).collect();
        let tombstones: Vec<_> = next
            .tombstones
            .iter()
            .map(|(id, generation)| json!({"key":id,"generation":generation}))
            .collect();
        query.push_str(" INSERT INTO memory_observation $observations; INSERT INTO memory_revision $revisions; INSERT INTO memory_relationship $relationships; INSERT INTO memory_job $jobs; INSERT INTO memory_tombstone $tombstones;");
        let mut partitions = Vec::new();
        if let Some(identity) = &next.embedding_identity {
            for claim in next.claims.values() {
                if let Some(vector) = claim.vectors.first() {
                    ensure!(
                        claim.embedding_identity == *identity,
                        "Incompatible embedding identity"
                    );
                    partitions.push(vector.len());
                }
            }
            partitions.sort_unstable();
            partitions.dedup();
            ensure!(
                partitions.len() <= 1,
                "Incompatible dimensions in memory store"
            );
            if let Some(&dimension) = partitions.first() {
                let table = vector_table(identity)?;
                // Define the partition outside the data transaction; failure leaves the old state intact.
                self.db.query(format!("DEFINE TABLE IF NOT EXISTS {table} SCHEMALESS; DEFINE INDEX IF NOT EXISTS vectors ON {table} FIELDS vector HNSW DIMENSION {dimension} DIST COSINE TYPE F32;")).await?.check()?;
                query.push_str(&format!(" DELETE {table}; INSERT INTO {table} $vectors;"));
            }
        }
        // Also clear the previous partition on purge/re-embedding, including when no claims remain.
        if let Some(identity) = &self.state.embedding_identity
            && (next.embedding_identity != self.state.embedding_identity || next.claims.is_empty())
        {
            query.push_str(&format!(" DELETE {};", vector_table(identity)?));
        }
        query.push_str(" COMMIT TRANSACTION;");
        let vectors: Vec<_> = next
            .claims
            .values()
            .flat_map(|claim| {
                claim.vectors.iter().enumerate().map(
                    |(chunk, vector)| json!({"memory_id":claim.id,"chunk":chunk,"vector":vector}),
                )
            })
            .collect();
        self.db
            .query(query)
            .bind(("payload", payload))
            .bind(("claims", claims))
            .bind(("observations", observations))
            .bind(("revisions", revisions))
            .bind(("relationships", relationships))
            .bind(("jobs", jobs))
            .bind(("tombstones", tombstones))
            .bind(("vectors", vectors))
            .await?
            .check()?;
        self.state = next;
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
fn vector_table(identity: &str) -> Result<String> {
    Ok(format!("memory_vector_{}", digest(&identity)?))
}
