// ABOUTME: Semantic acceptance, bounded reconciliation, recall, correction, and complete forgetting.
// ABOUTME: Classifier proposals cannot widen scope, establish verification, or bypass revision checks.
use crate::{
    context::{ConservativeCounter, pack_with_metadata},
    store::{Store, digest},
    types::*,
};
use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use codegraph_vector::{
    EmbeddingGenerator,
    reranking::{RerankDocument, Reranker},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tokio::sync::Mutex;
use uuid::Uuid;

#[async_trait]
pub trait Embedder: Send + Sync {
    fn available(&self) -> bool {
        true
    }
    fn identity(&self) -> Result<String>;
    async fn document(&self, text: &str) -> Result<Vec<Vec<f32>>>;
    async fn query(&self, text: &str) -> Result<Vec<f32>>;
}
#[async_trait]
impl Embedder for EmbeddingGenerator {
    fn identity(&self) -> Result<String> {
        digest(&self.input_identity()?)
    }
    async fn document(&self, text: &str) -> Result<Vec<Vec<f32>>> {
        ensure!(
            self.has_provider(),
            "Memory writes require a configured embedding provider"
        );
        Ok(self.embed_document_text(text).await?)
    }
    async fn query(&self, text: &str) -> Result<Vec<f32>> {
        ensure!(
            self.has_provider(),
            "Semantic recall requires a configured embedding provider"
        );
        Ok(self.generate_text_embedding(text).await?)
    }
}
#[async_trait]
pub trait Classifier: Send + Sync {
    fn identity(&self) -> String;
    fn finish_attempt(&self, _observation_id: &str) {}
    async fn extract(&self, observation: &Observation) -> Result<Vec<Proposal>>;
    async fn reconcile(
        &self,
        observation: &Observation,
        claims: Vec<Proposal>,
        candidates: Vec<Claim>,
    ) -> Result<Vec<Proposal>>;
}
#[derive(Clone)]
pub struct MemoryService {
    pub store: Arc<Mutex<Store>>,
    pub embedder: Arc<dyn Embedder>,
    pub classifier: Option<Arc<dyn Classifier>>,
    pub reranker: Option<Arc<dyn Reranker>>,
}
impl MemoryService {
    pub fn new(
        store: Store,
        embedder: Arc<dyn Embedder>,
        classifier: Option<Arc<dyn Classifier>>,
        reranker: Option<Arc<dyn Reranker>>,
    ) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            embedder,
            classifier,
            reranker,
        }
    }
    pub async fn write(&self, context: &ClientContext, request: WriteRequest) -> Result<Operation> {
        let started = std::time::Instant::now();
        validate_write(context, &request)?;
        let content = digest(&(context, &request))?;
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| {
                digest(&(
                    context.owner_id.clone(),
                    context.project_id.clone(),
                    context.session_id.clone(),
                    request.scope,
                    key,
                ))
            })
            .transpose()?;
        {
            let store = self.store.lock().await;
            if let Some(key) = &key
                && let Some((previous, operation)) = store.state.idempotency.get(key)
            {
                ensure!(
                    previous == &content,
                    "Idempotency key was already used with different input"
                );
                return store
                    .state
                    .operations
                    .get(operation)
                    .cloned()
                    .context("Forgotten operation");
            }
        }
        let identity = self.embedder.identity()?;
        let vectors = if request.asynchronous {
            Vec::new()
        } else {
            self.embedder.document(&request.statement).await?
        };
        validate_vectors(&vectors, !request.asynchronous)?;
        let mut store = self.store.lock().await;
        if let Some(key) = &key
            && let Some((previous, operation)) = store.state.idempotency.get(key)
        {
            ensure!(
                previous == &content,
                "Idempotency key was already used with different input"
            );
            return store
                .state
                .operations
                .get(operation)
                .cloned()
                .context("Forgotten operation");
        }
        let mut next = store.state.clone();
        ensure!(
            next.embedding_identity
                .as_ref()
                .is_none_or(|v| *v == identity),
            "Memory store uses an incompatible embedding identity; re-embed it explicitly"
        );
        next.embedding_identity = Some(identity.clone());
        let now = Utc::now();
        let observation = Observation {
            id: Uuid::new_v4().to_string(),
            context: context.clone(),
            request: request.clone(),
            recorded_at: now,
        };
        let claim = claim_from(&observation, request.statement.clone(), identity, vectors);
        let operation = Operation {
            id: Uuid::new_v4().to_string(),
            owner_id: context.owner_id.clone(),
            project_id: context.project_id.clone(),
            scope: request.scope,
            session_id: context.session_id.clone(),
            observation_id: observation.id.clone(),
            memory_ids: vec![claim.id.clone()],
            state: JobState::Pending,
            stage: if request.asynchronous {
                "embedding_pending"
            } else {
                "accepted"
            }
            .into(),
            embedding_ready: !request.asynchronous,
            attempts: 0,
            lease_until: None,
            retry_at: None,
            error: None,
            model_identity: self
                .classifier
                .as_ref()
                .map_or(String::new(), |v| v.identity()),
            correction_target: None,
        };
        next.claims.insert(claim.id.clone(), claim);
        next.observations
            .insert(observation.id.clone(), observation);
        next.operations
            .insert(operation.id.clone(), operation.clone());
        if let Some(key) = key {
            next.idempotency
                .insert(key, (content, operation.id.clone()));
        }
        store.commit(next).await?;
        tracing::info!(operation_id=%operation.id,embedding_ready=operation.embedding_ready,elapsed_ms=started.elapsed().as_millis(),"Memory observation accepted");
        Ok(operation)
    }

    pub async fn read(
        &self,
        context: &ClientContext,
        request: ReadRequest,
    ) -> Result<MemoryContext> {
        let mut effective_context = context.clone();
        effective_context
            .applicability
            .extend(request.applicability.clone());
        let context = &effective_context;
        ensure!(
            !request.query.trim().is_empty(),
            "Semantic recall requires a query"
        );
        ensure!(
            request.limit <= 100 && request.token_budget <= 100_000,
            "Memory read limits are too large"
        );
        let query = self.embedder.query(&request.query).await?;
        validate_vectors(std::slice::from_ref(&query), true)?;
        let identity = self.embedder.identity()?;
        let store = self.store.lock().await;
        ensure!(
            store
                .state
                .embedding_identity
                .as_ref()
                .is_none_or(|stored| *stored == identity),
            "Memory embedding identity is incompatible; re-embedding is required"
        );
        let pending_embeddings = store
            .state
            .claims
            .values()
            .any(|v| context.authorizes(v) && v.vectors.is_empty());
        let now = Utc::now();
        let at = request.as_of.unwrap_or(now);
        let mut eligible: BTreeMap<String, Claim> = store
            .state
            .claims
            .values()
            .filter(|v| is_eligible(v, context, &request, at, now))
            .map(|v| (v.id.clone(), v.clone()))
            .collect();
        if request.as_of.is_some() {
            for revision in &store.state.revisions {
                if is_eligible(&revision.claim, context, &request, at, now)
                    && !store.state.tombstones.contains_key(&revision.claim.id)
                {
                    let previous = eligible.get(&revision.claim.id);
                    if previous.is_none_or(|v| v.valid_from < revision.claim.valid_from) {
                        eligible.insert(revision.claim.id.clone(), revision.claim.clone());
                    }
                }
            }
        }
        ensure!(
            eligible.values().all(|claim| claim
                .vectors
                .iter()
                .all(|vector| vector.len() == query.len())),
            "Query embedding dimension is incompatible with stored memory vectors"
        );
        if eligible.is_empty() {
            let mut result = MemoryContext::default();
            if pending_embeddings {
                result.status = "partial".into();
                result.warnings.push("Authorized observations have embedding_pending; semantic coverage is incomplete".into());
            }
            if let Some(id) = &request.operation_id {
                result.operation = operation_for(&store.state, id, context).cloned();
            }
            return pack_with_metadata(
                Vec::new(),
                request.limit,
                request.token_budget,
                &ConservativeCounter,
                result,
            );
        }
        let (semantic, lexical) = if request.as_of.is_some() {
            // Historical embeddings are retained in revisions, not the current HNSW partition.
            (
                rank_semantic(&eligible, &query),
                rank_lexical(&eligible, &request.query),
            )
        } else {
            store
                .candidates(
                    &identity,
                    &request.query,
                    query.clone(),
                    eligible.keys().cloned().collect(),
                )
                .await?
        };
        let graph: Vec<_> = eligible
            .values()
            .filter(|v| {
                v.anchors
                    .iter()
                    .any(|a| request.node_ids.contains(&a.node_id))
            })
            .map(|v| v.id.clone())
            .take(100)
            .collect();
        let streams: Vec<_> = [semantic, lexical, graph]
            .into_iter()
            .filter(|v| !v.is_empty())
            .collect();
        let mut scores: BTreeMap<String, f64> = BTreeMap::new();
        for stream in &streams {
            for (rank, id) in stream.iter().enumerate() {
                *scores.entry(id.clone()).or_default() +=
                    1.0 / (60.0 + rank as f64 + 1.0) / streams.len() as f64;
            }
        }
        // Constraints are applicability-filtered before reserving their pack capacity.
        for claim in eligible.values().filter(|v| v.tier == Tier::Core) {
            scores.entry(claim.id.clone()).or_insert(0.0);
        }
        let operation = request
            .operation_id
            .as_ref()
            .and_then(|id| operation_for(&store.state, id, context))
            .cloned();
        drop(store);
        let mut ranked: Vec<_> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked.truncate(100);
        let mut warnings = Vec::new();
        if let Some(reranker) = &self.reranker
            && !ranked.is_empty()
        {
            let docs = ranked
                .iter()
                .map(|(id, _)| RerankDocument {
                    id: id.clone(),
                    text: eligible[id].statement.clone(),
                    metadata: Some(serde_json::json!({"revision":eligible[id].revision})),
                })
                .collect();
            match reranker.rerank(&request.query, docs, ranked.len()).await {
                Ok(results) => {
                    let offered: BTreeSet<_> = ranked.iter().map(|(id, _)| id.clone()).collect();
                    ensure!(
                        results.iter().all(|v| offered.contains(&v.id)),
                        "Reranker returned an unknown memory ID"
                    );
                    ranked = results
                        .into_iter()
                        .map(|v| (v.id, v.score as f64))
                        .collect();
                    for claim in eligible.values().filter(|v| v.tier == Tier::Core) {
                        if !ranked.iter().any(|(id, _)| id == &claim.id) {
                            ranked.push((claim.id.clone(), 0.0));
                        }
                    }
                }
                Err(_) => warnings.push(
                    "Configured reranker failed; returned declared partial fused ranking".into(),
                ),
            }
        }
        let entries = ranked
            .into_iter()
            .filter_map(|(id, _)| {
                let memory = eligible.remove(&id)?;
                // Require semantic relevance unless lexical/graph evidence or an applicable core constraint matched.
                if memory.tier != Tier::Core
                    && max_similarity(&memory, &query) < 0.15
                    && !request
                        .query
                        .split_whitespace()
                        .any(|w| memory.statement.to_lowercase().contains(&w.to_lowercase()))
                    && !memory
                        .anchors
                        .iter()
                        .any(|a| request.node_ids.contains(&a.node_id))
                {
                    return None;
                }
                let mut reasons = memory.review_reasons.clone();
                if memory.review_after.is_some_and(|v| v <= now) {
                    reasons.push("verification_due".into());
                }
                if !memory.conflicts.is_empty() {
                    reasons.push("unresolved_conflict".into());
                }
                reasons.sort();
                reasons.dedup();
                if !reasons.is_empty() && !request.include_needs_verification {
                    return None;
                }
                let code_context = anchor_context(&memory);
                Some(MemoryEntry {
                    reference: format!("memory:{}@{}", memory.id, memory.revision),
                    memory,
                    reasons,
                    code_context,
                })
            })
            .collect();
        let mut result = MemoryContext::default();
        if pending_embeddings {
            result.status = "partial".into();
            result.warnings.push(
                "Authorized observations have embedding_pending; semantic coverage is incomplete"
                    .into(),
            );
        }
        result.operation = operation;
        if !warnings.is_empty() {
            result.status = "partial".into();
            result.warnings.extend(warnings);
        }
        pack_with_metadata(
            entries,
            request.limit,
            request.token_budget,
            &ConservativeCounter,
            result,
        )
    }

    /// Execute one persisted job. Slow provider calls happen outside the store mutex.
    pub async fn process_next(&self) -> Result<bool> {
        if !self.embedder.available() {
            return Ok(false);
        }
        let (operation, observation) = {
            let mut store = self.store.lock().await;
            let now = Utc::now();
            let mut candidates: Vec<_> = store
                .state
                .operations
                .values()
                .filter(|job| {
                    matches!(job.state, JobState::Pending | JobState::ConfigRequired)
                        && job.retry_at.is_none_or(|v| v <= now)
                        || job.state == JobState::Running
                            && job.lease_until.is_none_or(|v| v <= now)
                })
                .cloned()
                .collect();
            candidates.sort_by_key(|job| {
                store
                    .state
                    .observations
                    .get(&job.observation_id)
                    .map(|v| v.recorded_at)
            });
            let mut blocked = store.state.clone();
            let mut changed = false;
            candidates.retain(|job| {
                let incompatible = self.classifier.as_ref().is_some_and(|classifier| {
                    !job.model_identity.is_empty() && job.model_identity != classifier.identity()
                });
                if incompatible {
                    if let Some(persisted) = blocked.operations.get_mut(&job.id)
                        && persisted.state != JobState::ConfigRequired
                    {
                        persisted.state = JobState::ConfigRequired;
                        persisted.error =
                            Some("Compatible classifier configuration required".into());
                        persisted.lease_until = None;
                        changed = true;
                    }
                    return false;
                }
                store.state.observations.contains_key(&job.observation_id)
                    && (self.classifier.is_some() || !job.embedding_ready)
            });
            if changed {
                store.commit(blocked).await?;
            }
            let Some(mut job) = candidates.into_iter().next() else {
                return Ok(false);
            };
            let Some(observation) = store.state.observations.get(&job.observation_id).cloned()
            else {
                return Ok(false);
            };
            if let Some(classifier) = &self.classifier {
                job.model_identity = classifier.identity();
            }
            job.state = JobState::Running;
            job.attempts += 1;
            job.lease_until = Some(now + Duration::seconds(180));
            let mut next = store.state.clone();
            next.operations.insert(job.id.clone(), job.clone());
            store.commit(next).await?;
            (job, observation)
        };
        tracing::info!(operation_id=%operation.id,attempt=operation.attempts,"Memory background attempt started");
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            self.process(&operation, &observation),
        )
        .await;
        if let Some(classifier) = &self.classifier {
            classifier.finish_attempt(&observation.id);
        }
        let error = match result {
            Ok(Ok(())) => {
                tracing::info!(operation_id=%operation.id,"Memory background attempt completed");
                return Ok(true);
            }
            Ok(Err(e)) => format!("{e:#}"),
            Err(_) => "Background processing deadline exceeded".into(),
        };
        let mut store = self.store.lock().await;
        let mut next = store.state.clone();
        if let Some(job) = next.operations.get_mut(&operation.id)
            && job.state == JobState::Running
            && job.attempts == operation.attempts
        {
            let permanent = error.contains("422")
                || error.contains("401")
                || error.contains("403")
                || error.contains("Invalid classifier")
                || error.contains("incompatible");
            job.state = if job.attempts >= 3 || permanent {
                JobState::Failed
            } else {
                JobState::Pending
            };
            job.retry_at =
                Some(Utc::now() + Duration::seconds(if job.attempts == 1 { 5 } else { 20 }));
            // Provider errors may contain request content/credentials; persist a safe classification only.
            job.error = Some(
                if permanent {
                    "Permanent provider or proposal validation failure"
                } else {
                    "Transient processing failure or deadline"
                }
                .into(),
            );
            job.lease_until = None;
            store.commit(next).await?;
        }
        Ok(true)
    }
    pub async fn reembed(&self, embedder: Arc<dyn Embedder>) -> Result<()> {
        let mut next = self.store.lock().await.state.clone();
        let generation = next.generation;
        let identity = embedder.identity()?;
        for claim in next
            .claims
            .values_mut()
            .chain(next.revisions.iter_mut().map(|v| &mut v.claim))
        {
            let vectors = embedder.document(&claim.statement).await?;
            validate_vectors(&vectors, true)?;
            claim.vectors = vectors;
            claim.embedding_identity = identity.clone();
        }
        next.embedding_identity = Some(identity);
        let mut store = self.store.lock().await;
        ensure!(
            store.state.generation == generation,
            "Memory changed during re-embedding; retry with the current generation"
        );
        store.commit(next).await?;
        Ok(())
    }

    async fn process(&self, job: &Operation, observation: &Observation) -> Result<()> {
        if !job.embedding_ready {
            let vectors = self
                .embedder
                .document(&observation.request.statement)
                .await?;
            validate_vectors(&vectors, true)?;
            let mut store = self.store.lock().await;
            let mut next = store.state.clone();
            ensure!(
                next.operations
                    .get(&job.id)
                    .is_some_and(|v| v.state == JobState::Running && v.attempts == job.attempts),
                "Operation was cancelled"
            );
            for id in &job.memory_ids {
                if let Some(claim) = next.claims.get_mut(id) {
                    claim.vectors = vectors.clone();
                }
            }
            let operation = next
                .operations
                .get_mut(&job.id)
                .context("Missing operation")?;
            operation.embedding_ready = true;
            operation.stage = "accepted".into();
            if self.classifier.is_none() {
                operation.state = JobState::Pending;
                operation.lease_until = None;
            }
            store.commit(next).await?;
        }
        let Some(classifier) = &self.classifier else {
            return Ok(());
        };
        let extracted = classifier.extract(observation).await?;
        validate_proposals(&extracted, observation, &[])?;
        let mut candidates = BTreeMap::new();
        let mut candidate_scores = BTreeMap::<String, f64>::new();
        let mut applicability = observation.context.applicability.clone();
        applicability.extend(observation.request.applicability.clone());
        for proposed in &extracted {
            let query = self.embedder.query(&proposed.statement).await?;
            let store = self.store.lock().await;
            let eligible: Vec<_> = store
                .state
                .claims
                .values()
                .filter(|v| {
                    observation.context.authorizes(v)
                        && v.scope == observation.request.scope
                        && v.applicability == applicability
                        && !matches!(v.lifecycle, Lifecycle::Provisional | Lifecycle::Retracted)
                        && !job.memory_ids.contains(&v.id)
                })
                .map(|v| v.id.clone())
                .collect();
            let (semantic, lexical) = store
                .candidates(
                    &self.embedder.identity()?,
                    &proposed.statement,
                    query,
                    eligible,
                )
                .await?;
            for stream in [semantic, lexical] {
                for (rank, id) in stream.into_iter().enumerate() {
                    if let Some(claim) = store.state.claims.get(&id) {
                        *candidate_scores.entry(id.clone()).or_default() +=
                            1.0 / (61.0 + rank as f64);
                        candidates.insert(id, claim.clone());
                    }
                }
            }
        }
        let mut ranked: Vec<_> = candidate_scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut used = serde_json::to_vec(&(observation, &extracted))?.len();
        ensure!(
            used <= 112 * 1024,
            "Invalid classifier: extraction exceeds reconciliation input budget"
        );
        let mut bounded = Vec::new();
        for (id, _) in ranked.into_iter().take(100) {
            let claim = candidates.remove(&id).context("Missing ranked candidate")?;
            let mut offered = claim.clone();
            offered.vectors.clear();
            for anchor in &mut offered.anchors {
                anchor.historical_snippet.clear();
            }
            let bytes = serde_json::to_vec(&offered)?.len();
            if used + bytes + 128 <= 112 * 1024 {
                used += bytes + 128;
                bounded.push(claim);
            }
        }
        let candidates = bounded;
        let proposals = classifier
            .reconcile(observation, extracted, candidates.clone())
            .await?;
        validate_proposals(&proposals, observation, &candidates)?;
        if job.correction_target.is_some() {
            ensure!(
                proposals.len() == 1,
                "Invalid classifier: targeted correction must produce one atomic claim; submit additional claims separately"
            );
        }
        let mut embedded = Vec::new();
        for proposal in proposals {
            let vectors = self.embedder.document(&proposal.statement).await?;
            validate_vectors(&vectors, true)?;
            embedded.push((proposal, vectors));
        }
        let mut store = self.store.lock().await;
        let mut next = store.state.clone();
        ensure!(
            next.operations
                .get(&job.id)
                .is_some_and(|v| v.state == JobState::Running && v.attempts == job.attempts),
            "Operation was cancelled"
        );
        ensure!(
            next.observations.contains_key(&observation.id),
            "Observation was forgotten"
        );
        ensure!(
            next.embedding_identity.as_deref() == Some(self.embedder.identity()?.as_str()),
            "Memory embedding generation changed; retry processing with compatible configuration"
        );
        let correction = if let Some((id, revision)) = &job.correction_target {
            let existing = next
                .claims
                .get(id)
                .context("Correction target was forgotten")?;
            ensure!(
                existing.revision == *revision,
                "Correction target revision changed; superseded job cannot overwrite it"
            );
            Some(existing.clone())
        } else {
            None
        };
        for candidate in &candidates {
            ensure!(
                next.claims
                    .get(&candidate.id)
                    .is_some_and(|v| v.revision == candidate.revision),
                "Concurrent candidate revision changed; retry reconciliation"
            );
        }
        for id in &job.memory_ids {
            next.claims.remove(id);
        }
        let changed_at = Utc::now();
        let mut revised_targets = BTreeSet::new();
        let mut ids = Vec::new();
        for (proposal, vectors) in embedded {
            let mut claim = claim_from(
                observation,
                proposal.statement.clone(),
                self.embedder.identity()?,
                vectors,
            );
            claim.lifecycle = Lifecycle::Active;
            claim.kind = proposal.kind;
            if proposal.code_related && claim.anchors.is_empty() {
                claim.grounding = Grounding::Unavailable;
            }
            claim.tier = if claim.scope == Scope::Session
                || proposal.working
                || proposal.kind == Kind::WorkingState
            {
                Tier::Working
            } else {
                Tier::Durable
            };
            claim.review_after = if claim.tier == Tier::Working {
                Some(Utc::now() + Duration::hours(24))
            } else {
                review_after(claim.kind, Utc::now())
            };
            claim.classification_identity = Some(classifier.identity());
            if let Some(previous) = &correction {
                claim.id = previous.id.clone();
                claim.revision = previous.revision;
                claim.expires_at = previous.expires_at;
                claim.anchors = previous.anchors.clone();
                claim.grounding = previous.grounding;
                claim.observation_ids = previous.observation_ids.clone();
                claim.recorded_at = previous.recorded_at;
                claim.valid_from = previous.valid_from;
                claim.review_reasons = previous.review_reasons.clone();
            }
            claim.evidence = proposal
                .evidence_indices
                .iter()
                .map(|&i| observation.request.evidence[i].clone())
                .collect();
            if proposal.core_candidate
                && observation.request.authorized_constraint
                && !claim.evidence.is_empty()
            {
                claim.tier = Tier::Core;
            }
            let equivalent = proposal
                .relationships
                .iter()
                .find(|v| v.kind == RelationshipKind::Equivalent);
            if let Some(relation) = equivalent
                && correction.is_none()
            {
                if revised_targets.insert(relation.memory_id.clone()) {
                    revise(
                        &mut next,
                        &relation.memory_id,
                        changed_at,
                        "equivalent_observation",
                    )?;
                }
                let existing = next
                    .claims
                    .get_mut(&relation.memory_id)
                    .context("Missing candidate")?;
                existing.observation_ids.push(observation.id.clone());
                for evidence in &claim.evidence {
                    if !existing
                        .evidence
                        .iter()
                        .any(|v| v.uri == evidence.uri && v.fingerprint == evidence.fingerprint)
                    {
                        existing.evidence.push(evidence.clone());
                    }
                }
                for anchor in &claim.anchors {
                    if !existing.anchors.iter().any(|v| {
                        v.graph_project_id == anchor.graph_project_id
                            && v.node_id == anchor.node_id
                            && v.span_hash == anchor.span_hash
                            && v.supporting == anchor.supporting
                    }) {
                        existing.anchors.push(anchor.clone());
                    }
                }
                if !existing.anchors.is_empty() {
                    existing.grounding = Grounding::Anchored;
                }
                ids.push(existing.id.clone());
                continue;
            }
            for relation in &proposal.relationships {
                if relation.kind == RelationshipKind::Contradictory
                    && revised_targets.insert(relation.memory_id.clone())
                {
                    revise(
                        &mut next,
                        &relation.memory_id,
                        changed_at,
                        "contradictory_observation",
                    )?;
                }
                let target = next
                    .claims
                    .get_mut(&relation.memory_id)
                    .context("Missing relationship target")?;
                match relation.kind {
                    RelationshipKind::Contradictory => {
                        target.conflicts.push(claim.id.clone());
                        target.evidence_state = EvidenceState::Disputed;
                        claim.conflicts.push(target.id.clone());
                        claim.evidence_state = EvidenceState::Disputed;
                    }
                    RelationshipKind::Correction => {
                        // A new assertion or newer timestamp is not authorization to supersede.
                        bail!(
                            "Invalid classifier: corrections require an explicit memory_update target"
                        );
                    }
                    _ => {}
                }
                next.relationships.push(Relationship {
                    source: claim.id.clone(),
                    target: relation.memory_id.clone(),
                    kind: relation.kind,
                    source_revision: Some(claim.revision),
                    target_revision: Some(relation.expected_revision),
                    operation_id: Some(job.id.clone()),
                    created_at: Some(changed_at),
                    evidence: claim.evidence.clone(),
                });
            }
            ids.push(claim.id.clone());
            next.claims.insert(claim.id.clone(), claim);
        }
        let operation = next
            .operations
            .get_mut(&job.id)
            .context("Missing operation")?;
        operation.state = JobState::Complete;
        operation.stage = "reconciled".into();
        operation.memory_ids = ids;
        operation.embedding_ready = true;
        operation.lease_until = None;
        operation.error = None;
        store.commit(next).await?;
        Ok(())
    }

    pub async fn status(&self, context: &ClientContext, id: &str) -> Result<Operation> {
        operation_for(&self.store.lock().await.state, id, context)
            .cloned()
            .context("Operation not found in authorized scope")
    }
    pub async fn grounding_checks(
        &self,
        context: &ClientContext,
        checks: Vec<GroundingCheck>,
    ) -> Result<()> {
        let mut store = self.store.lock().await;
        let mut next = store.state.clone();
        let mut changed = false;
        for check in checks {
            if let Some(claim) = next.claims.get_mut(&check.memory_id)
                && context.authorizes(claim)
                && claim.revision == check.expected_revision
                && (claim.grounding != check.grounding
                    || (!check.anchors.is_empty()
                        && serde_json::to_value(&claim.anchors)?
                            != serde_json::to_value(&check.anchors)?)
                    || !check
                        .reasons
                        .iter()
                        .all(|v| claim.review_reasons.contains(v)))
            {
                claim.grounding = check.grounding;
                if claim.anchors.is_empty() {
                    claim.anchors = check.anchors;
                }
                for reason in check.reasons {
                    if !claim.review_reasons.contains(&reason) {
                        claim.review_reasons.push(reason);
                    }
                }
                changed = true;
            }
        }
        if changed {
            store.commit(next).await?;
        }
        Ok(())
    }
    pub async fn retry(&self, context: &ClientContext, id: &str) -> Result<Operation> {
        let mut store = self.store.lock().await;
        let mut next = store.state.clone();
        operation_for(&next, id, context).context("Operation not found")?;
        let job = next.operations.get_mut(id).context("Operation not found")?;
        ensure!(
            matches!(job.state, JobState::Failed | JobState::ConfigRequired),
            "Only failed/configuration-blocked jobs can be retried"
        );
        job.state = JobState::Pending;
        job.attempts = 0;
        job.retry_at = None;
        job.error = None;
        if let Some(classifier) = &self.classifier {
            job.model_identity = classifier.identity();
        }
        let result = job.clone();
        store.commit(next).await?;
        Ok(result)
    }
    pub async fn update(
        &self,
        context: &ClientContext,
        request: UpdateRequest,
    ) -> Result<serde_json::Value> {
        let Some(id) = request.memory_id.clone() else {
            return self.selection(context, request.query.as_deref()).await;
        };
        ensure!(
            request.expected_revision.is_some(),
            "expected_revision is required for mutation"
        );
        ensure!(
            !(request.confirm && request.statement.is_some()),
            "Correct and confirm in separate operations"
        );
        ensure!(
            !(request.complete && request.statement.is_some()),
            "Correct and complete in separate operations"
        );
        if request.confirm {
            ensure!(
                !request.evidence.is_empty()
                    && request.evidence.iter().all(|v| v.verified_at.is_some()),
                "Confirmation requires explicitly re-verified evidence"
            );
        }
        let original = {
            let store = self.store.lock().await;
            let claim = store.state.claims.get(&id).context("Memory not found")?;
            ensure!(context.authorizes(claim), "Memory not found");
            claim.clone()
        };
        let vectors = if let Some(statement) = &request.statement {
            ensure!(
                !statement.trim().is_empty() && statement.len() <= 64 * 1024,
                "Empty or oversized correction"
            );
            let vectors = self.embedder.document(statement).await?;
            validate_vectors(&vectors, true)?;
            Some(vectors)
        } else {
            None
        };
        let mut store = self.store.lock().await;
        let mut next = store.state.clone();
        let claim = next.claims.get_mut(&id).context("Memory not found")?;
        ensure!(
            claim.embedding_identity == self.embedder.identity()?,
            "Memory embedding identity changed; use compatible configuration"
        );
        ensure!(
            context.authorizes(claim)
                && Some(claim.revision) == request.expected_revision
                && claim.revision == original.revision,
            "Revision conflict"
        );
        let now = Utc::now();
        if request.statement.is_some()
            || request.confirm
            || request.complete
            || request.ttl_seconds.is_some()
        {
            let mut previous = claim.clone();
            previous.valid_until = Some(now);
            next.revisions.push(Revision {
                claim: previous,
                changed_at: now,
                reason: if request.confirm {
                    "confirmation"
                } else {
                    "correction"
                }
                .into(),
            });
            claim.revision += 1;
            claim.valid_from = now;
        }
        let has_correction = request.statement.is_some();
        if let Some(statement) = request.statement {
            claim.statement = statement;
            claim.vectors = vectors.context("Missing correction vectors")?;
            claim.evidence = request.evidence.clone();
            claim.evidence_state = EvidenceState::Reported;
            claim.confirmed_at = None;
            claim.lifecycle = Lifecycle::Provisional;
            claim
                .review_reasons
                .push("corrected_claim_requires_verification".into());
        }
        if request.confirm {
            claim.evidence = request.evidence;
            claim.evidence_state = if claim.conflicts.is_empty() {
                EvidenceState::Verified
            } else {
                EvidenceState::Disputed
            };
            claim.confirmed_at = Some(now);
            claim.review_after = review_after(claim.kind, now);
            claim.review_reasons.retain(|v| v == "unresolved_conflict");
            if claim.kind == Kind::Procedure && request.reusable && claim.conflicts.is_empty() {
                claim.tier = Tier::Core;
            }
        }
        if request.complete {
            ensure!(
                claim.tier == Tier::Working || claim.scope == Scope::Session,
                "Completion applies only to session/working-state memories"
            );
            claim.lifecycle = Lifecycle::Archived;
            claim.tier = Tier::Archive;
        }
        if let Some(ttl) = request.ttl_seconds {
            ensure!(ttl <= i64::MAX as u64 / 1000, "TTL is too large");
            claim.expires_at = Some(
                now.checked_add_signed(Duration::seconds(ttl as i64))
                    .context("TTL exceeds supported date range")?,
            );
        }
        if let Some(useful) = request.useful {
            let key = request
                .feedback_id
                .context("Feedback requires an idempotent feedback_id")?;
            if !claim.feedback_ids.contains(&key) {
                claim.usefulness += if useful { 1 } else { -1 };
                claim.feedback_ids.push(key);
            }
        }
        let mut result = claim.clone();
        result.vectors.clear();
        let mut operation_id = None;
        if has_correction {
            let observation_id = Uuid::new_v4().to_string();
            let request: WriteRequest = serde_json::from_value(
                serde_json::json!({"statement":claim.statement,"scope":claim.scope,"evidence":claim.evidence,"applicability":claim.applicability,"anchors":claim.anchors,"code_related":claim.grounding!=Grounding::NotApplicable}),
            )?;
            claim.observation_ids.push(observation_id.clone());
            let observation = Observation {
                id: observation_id.clone(),
                context: context.clone(),
                request,
                recorded_at: now,
            };
            let operation = Operation {
                id: Uuid::new_v4().to_string(),
                owner_id: context.owner_id.clone(),
                project_id: context.project_id.clone(),
                scope: claim.scope,
                session_id: context.session_id.clone(),
                observation_id,
                memory_ids: vec![claim.id.clone()],
                state: JobState::Pending,
                stage: "correction_accepted".into(),
                embedding_ready: true,
                attempts: 0,
                lease_until: None,
                retry_at: None,
                error: None,
                model_identity: self
                    .classifier
                    .as_ref()
                    .map_or(String::new(), |v| v.identity()),
                correction_target: Some((claim.id.clone(), claim.revision)),
            };
            operation_id = Some(operation.id.clone());
            next.observations
                .insert(observation.id.clone(), observation);
            next.operations.insert(operation.id.clone(), operation);
        }
        store.commit(next).await?;
        let mut result = serde_json::to_value(result)?;
        if let Some(id) = operation_id {
            result["operation_id"] = serde_json::json!(id);
        }
        Ok(result)
    }
    async fn selection(
        &self,
        context: &ClientContext,
        query: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut request = ReadRequest::new(query.context("Specify memory_id or a semantic query")?);
        request.include_provisional = true;
        request.token_budget = 20_000;
        let context = self.read(context, request).await?;
        Ok(
            serde_json::json!({"status":"needs_selection","candidates":context,"instruction":"Select memory_id and expected_revision; semantic similarity does not authorize mutation"}),
        )
    }
    pub async fn delete(
        &self,
        context: &ClientContext,
        request: DeleteRequest,
    ) -> Result<serde_json::Value> {
        let Some(id) = request.memory_id else {
            return self.selection(context, request.query.as_deref()).await;
        };
        let mut store = self.store.lock().await;
        let mut next = store.state.clone();
        let claim = next.claims.get(&id).context("Memory not found")?;
        ensure!(context.authorizes(claim), "Memory not found");
        ensure!(
            Some(claim.revision) == request.expected_revision,
            "Revision conflict; expected_revision required"
        );
        // Observations deriving multiple claims cannot retain the forgotten claim in raw source.
        let observations: BTreeSet<_> = claim.observation_ids.iter().cloned().collect();
        let ids: BTreeSet<_> = std::iter::once(id).collect();
        for id in &ids {
            if let Some(claim) = next.claims.remove(id) {
                next.tombstones.insert(id.clone(), claim.revision + 1);
            }
        }
        next.observations.retain(|id, _| !observations.contains(id));
        next.revisions.retain(|v| !ids.contains(&v.claim.id));
        for revision in &mut next.revisions {
            revision
                .claim
                .observation_ids
                .retain(|o| !observations.contains(o));
        }
        next.relationships
            .retain(|v| !ids.contains(&v.source) && !ids.contains(&v.target));
        for claim in next.claims.values_mut() {
            claim.conflicts.retain(|v| !ids.contains(v));
            claim.observation_ids.retain(|o| !observations.contains(o));
            if claim.conflicts.is_empty() && claim.evidence_state == EvidenceState::Disputed {
                claim.evidence_state = EvidenceState::Reported;
            }
        }
        for operation in next
            .operations
            .values_mut()
            .filter(|v| observations.contains(&v.observation_id))
        {
            operation.state = JobState::Cancelled;
            operation.observation_id.clear();
            operation.memory_ids.clear();
            operation.error = None;
            operation.stage = "forgotten".into();
            operation.lease_until = None;
        }
        store.commit(next).await?;
        Ok(
            serde_json::json!({"status":"forgotten","memory_ids":ids,"source_derivatives_purged":true}),
        )
    }
}

fn revise(state: &mut State, id: &str, at: chrono::DateTime<Utc>, reason: &str) -> Result<()> {
    let claim = state
        .claims
        .get_mut(id)
        .context("Missing revision target")?;
    let mut previous = claim.clone();
    previous.valid_until = Some(at);
    state.revisions.push(Revision {
        claim: previous,
        changed_at: at,
        reason: reason.into(),
    });
    claim.revision += 1;
    claim.valid_from = at;
    Ok(())
}

fn validate_write(context: &ClientContext, request: &WriteRequest) -> Result<()> {
    ensure!(
        !context.owner_id.is_empty() && !context.project_id.is_empty(),
        "Owner/project identity required"
    );
    ensure!(
        !request.statement.trim().is_empty() && request.statement.len() <= 64 * 1024,
        "Observation must contain 1..65536 UTF-8 bytes"
    );
    ensure!(
        serde_json::to_vec(request)?.len() <= 96 * 1024,
        "Observation evidence/anchor wrapper exceeds the write budget"
    );
    ensure!(
        request.scope != Scope::Session || context.session_id.is_some(),
        "Session scope requires explicit session identity"
    );
    ensure!(
        request
            .ttl_seconds
            .is_none_or(|v| v <= i64::MAX as u64 / 1000),
        "TTL is too large"
    );
    if let Some(ttl) = request.ttl_seconds {
        ensure!(
            Utc::now()
                .checked_add_signed(Duration::seconds(ttl as i64))
                .is_some(),
            "TTL exceeds supported date range"
        );
    }
    ensure!(
        request.evidence.len() <= 64 && request.anchors.len() <= 32,
        "Too many evidence references or anchors"
    );
    Ok(())
}
fn claim_from(
    observation: &Observation,
    statement: String,
    identity: String,
    vectors: Vec<Vec<f32>>,
) -> Claim {
    let request = &observation.request;
    let context = &observation.context;
    let now = observation.recorded_at;
    let mut applicability = context.applicability.clone();
    applicability.extend(request.applicability.clone());
    Claim {
        id: Uuid::new_v4().to_string(),
        revision: 1,
        statement,
        owner_id: context.owner_id.clone(),
        project_id: context.project_id.clone(),
        session_id: context.session_id.clone(),
        scope: request.scope,
        kind: Kind::Unclassified,
        tier: if request.scope == Scope::Session {
            Tier::Working
        } else {
            Tier::Durable
        },
        lifecycle: Lifecycle::Provisional,
        evidence_state: EvidenceState::Reported,
        grounding: if !request.anchors.is_empty() {
            Grounding::Anchored
        } else if request.code_related {
            Grounding::Unavailable
        } else {
            Grounding::NotApplicable
        },
        evidence: request.evidence.clone(),
        anchors: request.anchors.clone(),
        observation_ids: vec![observation.id.clone()],
        applicability,
        recorded_at: now,
        observed_at: now,
        valid_from: now,
        valid_until: None,
        expires_at: request
            .ttl_seconds
            .map(|v| now + Duration::seconds(v as i64)),
        review_after: None,
        confirmed_at: None,
        review_reasons: Vec::new(),
        conflicts: Vec::new(),
        classification_identity: None,
        embedding_identity: identity,
        vectors,
        usefulness: 0,
        feedback_ids: Vec::new(),
    }
}
fn validate_vectors(vectors: &[Vec<f32>], required: bool) -> Result<()> {
    ensure!(
        !required || !vectors.is_empty(),
        "Embedding provider returned no vectors"
    );
    if let Some(first) = vectors.first() {
        ensure!(
            !first.is_empty()
                && vectors
                    .iter()
                    .all(|v| v.len() == first.len() && v.iter().all(|n| n.is_finite()))
                && vectors.iter().all(|v| v.iter().any(|n| *n != 0.0)),
            "Invalid embedding dimensions or values"
        );
    }
    Ok(())
}
fn validate_proposals(
    proposals: &[Proposal],
    observation: &Observation,
    candidates: &[Claim],
) -> Result<()> {
    ensure!(
        !proposals.is_empty() && proposals.len() <= 32,
        "Invalid classifier: expected 1..32 atomic claims"
    );
    for proposal in proposals {
        ensure!(
            !proposal.statement.trim().is_empty() && proposal.statement.len() <= 64 * 1024,
            "Invalid classifier: empty or oversized claim"
        );
        ensure!(
            proposal
                .evidence_indices
                .iter()
                .all(|&i| i < observation.request.evidence.len()),
            "Invalid classifier: fabricated evidence"
        );
        ensure!(
            proposal.relationships.iter().all(|r| candidates
                .iter()
                .any(|c| c.id == r.memory_id && c.revision == r.expected_revision)),
            "Invalid classifier: unknown candidate or revision"
        );
    }
    Ok(())
}
fn review_after(kind: Kind, now: chrono::DateTime<Utc>) -> Option<chrono::DateTime<Utc>> {
    match kind {
        Kind::WorkingState => Some(now + Duration::hours(24)),
        Kind::Fact | Kind::Procedure => Some(now + Duration::days(30)),
        Kind::Decision | Kind::Episode => Some(now + Duration::days(90)),
        _ => None,
    }
}
fn is_eligible(
    claim: &Claim,
    context: &ClientContext,
    request: &ReadRequest,
    at: chrono::DateTime<Utc>,
    now: chrono::DateTime<Utc>,
) -> bool {
    context.authorizes(claim)
        && request.scope.is_none_or(|v| v == claim.scope)
        && (request.kinds.is_empty() || request.kinds.contains(&claim.kind))
        && request.node_id.as_ref().is_none_or(|id| {
            claim
                .anchors
                .iter()
                .any(|a| a.node_id.trim_start_matches("nodes:") == id.trim_start_matches("nodes:"))
        })
        && request.symbol.as_ref().is_none_or(|symbol| {
            claim.anchors.iter().any(|a| a.symbol == *symbol) || claim.statement.contains(symbol)
        })
        && !claim.vectors.is_empty()
        && claim
            .applicability
            .iter()
            .all(|(key, value)| context.applicability.get(key) == Some(value))
        && claim.lifecycle != Lifecycle::Retracted
        && (claim.lifecycle != Lifecycle::Provisional
            || request.include_provisional
            || context.session_id.is_some() && context.session_id == claim.session_id)
        && (!matches!(claim.lifecycle, Lifecycle::Superseded | Lifecycle::Archived)
            || request.include_archived
            || request.as_of.is_some())
        && (request.include_expired || claim.expires_at.is_none_or(|v| v > at))
        && claim.valid_from <= at
        && (request.as_of.is_none() || claim.valid_until.is_none_or(|v| v > at))
        && request.since.is_none_or(|v| claim.recorded_at >= v)
        && (request.include_stale
            || request.include_needs_verification
            || claim.review_reasons.is_empty()
                && claim.conflicts.is_empty()
                && claim.review_after.is_none_or(|v| v > now))
}
fn operation_for<'a>(state: &'a State, id: &str, context: &ClientContext) -> Option<&'a Operation> {
    state.operations.get(id).filter(|job| {
        job.owner_id == context.owner_id
            && (job.scope == Scope::User || job.project_id == context.project_id)
            && (job.scope != Scope::Session
                || context.session_id.is_some() && context.session_id == job.session_id)
    })
}
pub fn max_similarity(claim: &Claim, query: &[f32]) -> f32 {
    claim
        .vectors
        .iter()
        .filter(|v| v.len() == query.len())
        .map(|v| {
            let dot: f32 = v.iter().zip(query).map(|(a, b)| a * b).sum();
            let a: f32 = v.iter().map(|n| n * n).sum::<f32>().sqrt();
            let b: f32 = query.iter().map(|n| n * n).sum::<f32>().sqrt();
            if a * b == 0.0 { 0.0 } else { dot / (a * b) }
        })
        .fold(-1.0, f32::max)
}
fn rank_semantic(claims: &BTreeMap<String, Claim>, query: &[f32]) -> Vec<String> {
    let mut ranked: Vec<_> = claims
        .values()
        .map(|v| (v.id.clone(), max_similarity(v, query)))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.into_iter().take(100).map(|v| v.0).collect()
}
fn rank_lexical(claims: &BTreeMap<String, Claim>, query: &str) -> Vec<String> {
    claims
        .values()
        .filter(|v| {
            query
                .split_whitespace()
                .any(|w| v.statement.to_lowercase().contains(&w.to_lowercase()))
        })
        .map(|v| v.id.clone())
        .take(100)
        .collect()
}
fn anchor_context(claim: &Claim) -> CodeContext {
    let mut context = CodeContext::default();
    for anchor in &claim.anchors {
        context.nodes.push(serde_json::json!({"id":anchor.node_id,"symbol":anchor.symbol,"path":anchor.path,"snapshot":"historical","graph_project_id":anchor.graph_project_id}));
        context.paths.push(serde_json::json!({"kind":if anchor.supporting{"supporting"}else{"retrieval"},"anchor":anchor.node_id,"code_hops":0}));
        if !anchor.historical_snippet.is_empty() {
            context.snippets.push(serde_json::json!({"node_id":anchor.node_id,"snapshot":"historical","text":anchor.historical_snippet,"start_line":anchor.start_line,"end_line":anchor.end_line,"span_hash":anchor.span_hash}));
        }
    }
    context
}
