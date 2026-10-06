// ABOUTME: Typed SurrealDB records and native provenance/relationship edges.
// ABOUTME: Stable identities allow incremental commits without rewriting unrelated HNSW entries.
use crate::{store::digest, types::*};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use surrealdb::types::{RecordId, SurrealValue, ToSql, Value};

#[derive(Clone, PartialEq)]
pub(crate) struct Row {
    pub record: RecordId,
    pub content: Value,
    pub endpoints: Option<(RecordId, RecordId)>,
}
pub(crate) type Rows = BTreeMap<(String, String), Row>;

fn native_json(value: serde_json::Value, field: &str) -> Result<Value> {
    if let Some(integer) = value.as_u64() {
        ensure!(
            integer <= i64::MAX as u64,
            "Memory integer exceeds SurrealDB's signed range"
        );
    }
    if field == "applicability" {
        return Ok(value.into_value());
    }
    Ok(match value {
        serde_json::Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| Ok((key.clone(), native_json(value, &key)?)))
                .collect::<Result<_>>()?,
        ),
        serde_json::Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|value| native_json(value, field))
                .collect::<Result<_>>()?,
        ),
        serde_json::Value::String(text)
            if matches!(
                field,
                "recorded_at"
                    | "observed_at"
                    | "valid_from"
                    | "valid_until"
                    | "expires_at"
                    | "review_after"
                    | "confirmed_at"
                    | "verified_at"
                    | "changed_at"
                    | "created_at"
                    | "lease_until"
                    | "retry_at"
            ) =>
        {
            Value::Datetime(text.parse()?)
        }
        value => value.into_value(),
    })
}
pub(crate) fn serde_json_value(value: Value) -> Result<serde_json::Value> {
    let value = match value {
        Value::RecordId(id) => Value::String(id.to_sql()),
        Value::Datetime(time) => Value::String(time.into_inner().to_rfc3339()),
        Value::Array(items) => {
            return items
                .into_iter()
                .map(serde_json_value)
                .collect::<Result<Vec<_>>>()
                .map(serde_json::Value::Array);
        }
        Value::Object(fields) => {
            return fields
                .into_iter()
                .map(|(key, value)| Ok((key, serde_json_value(value)?)))
                .collect::<Result<_>>()
                .map(serde_json::Value::Object);
        }
        value => value,
    };
    Ok(<serde_json::Value as SurrealValue>::from_value(value)?)
}
fn insert(
    rows: &mut Rows,
    table: &str,
    key: String,
    content: serde_json::Value,
    endpoints: Option<(RecordId, RecordId)>,
) -> Result<()> {
    let row = Row {
        record: RecordId::new(table, key.clone()),
        content: native_json(content, "")?,
        endpoints,
    };
    rows.insert((table.into(), key), row);
    Ok(())
}
fn object<T: Serialize>(value: &T, identity: &str) -> Result<serde_json::Value> {
    let mut object = serde_json::to_value(value)?;
    let fields = object.as_object_mut().context("record must be an object")?;
    if let Some(id) = fields.remove("id") {
        fields.insert(identity.into(), id);
    }
    Ok(object)
}
fn add_claim_details(rows: &mut Rows, claim: &Claim, state: &State) -> Result<()> {
    let from = RecordId::new("memory_claim", claim.id.clone());
    for (index, evidence) in claim.evidence.iter().enumerate() {
        let mut data = native_json(serde_json::to_value(evidence)?, "")?;
        if let Value::Object(fields) = &mut data {
            fields.insert("claim", Value::RecordId(from.clone()));
            fields.insert("revision", claim.revision.into_value());
        }
        let key = digest(&(&claim.id, claim.revision, index))?;
        rows.insert(
            ("memory_evidence".into(), key.clone()),
            Row {
                record: RecordId::new("memory_evidence", key),
                content: data,
                endpoints: None,
            },
        );
    }
    for anchor in &claim.anchors {
        let locator_key = digest(anchor)?;
        let locator = RecordId::new("memory_code_locator", locator_key.clone());
        insert(
            rows,
            "memory_code_locator",
            locator_key.clone(),
            serde_json::to_value(anchor)?,
            None,
        )?;
        insert(
            rows,
            "memory_link",
            digest(&(&claim.id, claim.revision, &locator_key))?,
            json!({"revision":claim.revision,"role":if anchor.supporting {"evidence"} else {"related"}}),
            Some((from.clone(), locator)),
        )?;
    }
    for id in &claim.observation_ids {
        if state.observations.contains_key(id) {
            insert(
                rows,
                "memory_derived_from",
                digest(&(&claim.id, claim.revision, id))?,
                json!({"revision":claim.revision}),
                Some((
                    from.clone(),
                    RecordId::new("memory_observation", id.clone()),
                )),
            )?;
        }
    }
    Ok(())
}
pub(crate) fn rows(state: &State) -> Result<Rows> {
    let mut rows = Rows::new();
    for claim in state.claims.values() {
        insert(
            &mut rows,
            "memory_claim",
            claim.id.clone(),
            object(claim, "memory_id")?,
            None,
        )?;
        add_claim_details(&mut rows, claim, state)?;
        for (chunk, vector) in claim.vectors.iter().enumerate() {
            let table = crate::store::vector_table(&claim.embedding_identity)?;
            let key = digest(&(&claim.id, chunk))?;
            let mut content = native_json(
                json!({"memory_id":claim.id,"chunk":chunk,"vector":vector}),
                "",
            )?;
            if let Value::Object(fields) = &mut content {
                fields.insert(
                    "claim",
                    Value::RecordId(RecordId::new("memory_claim", claim.id.clone())),
                );
            }
            rows.insert(
                (table.clone(), key.clone()),
                Row {
                    record: RecordId::new(table, key),
                    content,
                    endpoints: None,
                },
            );
        }
    }
    for observation in state.observations.values() {
        insert(
            &mut rows,
            "memory_observation",
            observation.id.clone(),
            object(observation, "observation_id")?,
            None,
        )?;
    }
    for operation in state.operations.values() {
        insert(
            &mut rows,
            "memory_job",
            operation.id.clone(),
            object(operation, "operation_id")?,
            None,
        )?;
    }
    for revision in &state.revisions {
        let mut content = serde_json::to_value(revision)?;
        content["memory_id"] = revision.claim.id.clone().into();
        content["revision"] = revision.claim.revision.into();
        content["statement"] = revision.claim.statement.clone().into();
        insert(
            &mut rows,
            "memory_revision",
            digest(&(&revision.claim.id, revision.claim.revision))?,
            content,
            None,
        )?;
        add_claim_details(&mut rows, &revision.claim, state)?;
    }
    for relation in &state.relationships {
        insert(
            &mut rows,
            "memory_relationship",
            digest(relation)?,
            serde_json::to_value(relation)?,
            Some((
                RecordId::new("memory_claim", relation.source.clone()),
                RecordId::new("memory_claim", relation.target.clone()),
            )),
        )?;
    }
    for (key, (content_digest, operation_id)) in &state.idempotency {
        insert(
            &mut rows,
            "memory_idempotency",
            key.clone(),
            json!({"digest":key,"content_digest":content_digest,"operation_id":operation_id}),
            None,
        )?;
    }
    for (id, generation) in &state.tombstones {
        insert(
            &mut rows,
            "memory_tombstone",
            id.clone(),
            json!({"memory_id":id,"generation":generation}),
            None,
        )?;
    }
    Ok(rows)
}
impl Row {
    pub fn binding(&self) -> Value {
        let mut fields = BTreeMap::from([
            ("id".into(), Value::RecordId(self.record.clone())),
            ("content".into(), self.content.clone()),
        ]);
        if let Some((from, to)) = &self.endpoints {
            fields.insert("in".into(), Value::RecordId(from.clone()));
            fields.insert("out".into(), Value::RecordId(to.clone()));
        }
        Value::Object(fields.into_iter().collect())
    }
}
