// ABOUTME: Offline semantic memory regressions with isolated stores and deterministic provider mocks.
// ABOUTME: Exercises persistence, reconciliation, scope, readiness, lifecycle, and deletion races.
use anyhow::Result;
use async_trait::async_trait;
use codegraph_memory::{store::Store, *};
use std::sync::Arc;

struct Embeddings(&'static str);
#[async_trait]
impl Embedder for Embeddings {
    fn identity(&self) -> Result<String> {
        Ok(self.0.into())
    }
    async fn document(&self, text: &str) -> Result<Vec<Vec<f32>>> {
        Ok(vec![vector(text)])
    }
    async fn query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(vector(text))
    }
}
fn vector(text: &str) -> Vec<f32> {
    let t = text.to_lowercase();
    if t.contains("scoring") || t.contains("parallel") {
        vec![1.0, 0.0, 0.0]
    } else if t.contains("cat") {
        vec![0.0, 1.0, 0.0]
    } else {
        vec![0.0, 0.0, 1.0]
    }
}
struct Classification;
#[async_trait]
impl Classifier for Classification {
    fn identity(&self) -> String {
        "mock-v1".into()
    }
    async fn extract(&self, observation: &Observation) -> Result<Vec<Proposal>> {
        Ok(observation
            .request
            .statement
            .split('|')
            .map(|statement| Proposal {
                statement: statement.trim().into(),
                kind: Kind::Decision,
                working: false,
                core_candidate: false,
                code_related: false,
                relationships: Vec::new(),
                evidence_indices: (0..observation.request.evidence.len()).collect(),
            })
            .collect())
    }
    async fn reconcile(
        &self,
        _: &Observation,
        mut claims: Vec<Proposal>,
        candidates: Vec<Claim>,
    ) -> Result<Vec<Proposal>> {
        for claim in &mut claims {
            for candidate in &candidates {
                if candidate.statement == claim.statement {
                    claim.relationships.push(ProposedRelationship {
                        memory_id: candidate.id.clone(),
                        expected_revision: candidate.revision,
                        kind: RelationshipKind::Equivalent,
                    });
                } else if candidate.statement.contains("parallel")
                    && claim.statement.contains("parallel")
                {
                    claim.relationships.push(ProposedRelationship {
                        memory_id: candidate.id.clone(),
                        expected_revision: candidate.revision,
                        kind: RelationshipKind::Contradictory,
                    });
                }
            }
        }
        Ok(claims)
    }
}
fn context(project: &str, session: Option<&str>) -> ClientContext {
    ClientContext {
        owner_id: "alice".into(),
        project_id: project.into(),
        session_id: session.map(str::to_string),
        task_id: None,
        context_epoch: None,
        applicability: Default::default(),
    }
}
fn write(text: &str) -> WriteRequest {
    serde_json::from_value(serde_json::json!({"statement":text})).unwrap()
}
fn read(text: &str) -> ReadRequest {
    let mut read = ReadRequest::new(text);
    read.token_budget = 20_000;
    read
}
async fn service() -> MemoryService {
    MemoryService::new(
        Store::open(None).await.unwrap(),
        Arc::new(Embeddings("vectors-v1")),
        Some(Arc::new(Classification)),
        None,
    )
}
async fn active(service: &MemoryService, context: &ClientContext, text: &str) -> Operation {
    let operation = service.write(context, write(text)).await.unwrap();
    service.process_next().await.unwrap();
    service.status(context, &operation.id).await.unwrap()
}

#[tokio::test]
async fn semantic_acceptance_reconciliation_and_unanchored_recall() {
    let service = service().await;
    let context = context("project", Some("session"));
    let accepted = service
        .write(&context, write("Keep scoring deterministic"))
        .await
        .unwrap();
    assert!(accepted.embedding_ready);
    assert_eq!(accepted.state, JobState::Pending);
    let pending = service.read(&context, read("scoring")).await.unwrap();
    assert_eq!(pending.memories[0].memory.lifecycle, Lifecycle::Provisional);
    service.process_next().await.unwrap();
    let completed = service.status(&context, &accepted.id).await.unwrap();
    assert_eq!(completed.state, JobState::Complete);
    let recalled = service
        .read(&context, read("scoring constraints"))
        .await
        .unwrap();
    assert_eq!(recalled.memories.len(), 1);
    assert_eq!(
        recalled.memories[0].memory.grounding,
        Grounding::NotApplicable
    );
    assert!(recalled.needs_verification.is_empty());
}
#[tokio::test]
async fn idempotency_and_concurrent_acceptance() {
    let service = service().await;
    let context = context("project", None);
    let mut request = write("scoring constraints");
    request.idempotency_key = Some("write-1".into());
    let (first, second) = tokio::join!(
        service.write(&context, request.clone()),
        service.write(&context, request.clone())
    );
    assert_eq!(first.unwrap().id, second.unwrap().id);
    request.statement = "different scoring claim".into();
    assert!(
        service
            .write(&context, request)
            .await
            .unwrap_err()
            .to_string()
            .contains("different input")
    );
    assert_eq!(service.store.lock().await.state.observations.len(), 1);
}
#[tokio::test]
async fn provisional_visibility_requires_originating_session_or_explicit_filter() {
    let service = service().await;
    let origin = context("project", Some("a"));
    service.write(&origin, write("scoring")).await.unwrap();
    assert!(
        service
            .read(&context("project", Some("b")), read("scoring"))
            .await
            .unwrap()
            .memories
            .is_empty()
    );
    let mut query = read("scoring");
    query.include_provisional = true;
    assert_eq!(
        service
            .read(&context("project", None), query)
            .await
            .unwrap()
            .memories
            .len(),
        1
    );
}
#[tokio::test]
async fn scopes_filter_before_knn_and_never_reconcile_across_projects() {
    let service = service().await;
    let alice = context("project-a", None);
    let other = context("project-b", None);
    for _ in 0..105 {
        let mut request = write("cat nearest foreign record");
        request.asynchronous = true;
        service.write(&other, request).await.unwrap();
    }
    {
        let mut store = service.store.lock().await;
        let mut state = store.state.clone();
        for claim in state.claims.values_mut() {
            claim.lifecycle = Lifecycle::Active;
            claim.vectors = vec![vec![0.0, 1.0, 0.0]];
        }
        for operation in state.operations.values_mut() {
            operation.state = JobState::Complete;
        }
        store.commit(state).await.unwrap();
    }
    let operation = active(&service, &alice, "scoring project constraint").await;
    assert_eq!(operation.state, JobState::Complete, "{:?}", operation);
    let result = service.read(&alice, read("scoring")).await.unwrap();
    assert_eq!(result.memories.len(), 1, "{result:?}");
    assert!(
        service
            .read(&other, read("scoring"))
            .await
            .unwrap()
            .memories
            .is_empty()
    );
    let mut bob = alice.clone();
    bob.owner_id = "bob".into();
    assert!(
        service
            .read(&bob, read("scoring"))
            .await
            .unwrap()
            .memories
            .is_empty()
    );
}
#[tokio::test]
async fn equivalent_assertions_do_not_verify_and_conflicts_are_warnings() {
    let service = service().await;
    let context = context("project", None);
    active(&service, &context, "parallel scoring is unsafe").await;
    active(&service, &context, "parallel scoring is unsafe").await;
    assert_eq!(service.store.lock().await.state.claims.len(), 1);
    let recalled = service
        .read(&context, read("parallel scoring"))
        .await
        .unwrap();
    assert_eq!(
        recalled.memories[0].memory.evidence_state,
        EvidenceState::Reported
    );
    active(&service, &context, "parallel scoring is safe").await;
    let recalled = service
        .read(&context, read("parallel scoring"))
        .await
        .unwrap();
    assert!(recalled.memories.is_empty());
    assert_eq!(recalled.needs_verification.len(), 2);
}
#[tokio::test]
async fn changed_evidence_is_returned_when_stale_is_excluded() {
    let service = service().await;
    let context = context("project", None);
    let operation = active(&service, &context, "scoring constraints").await;
    service
        .grounding_checks(
            &context,
            vec![GroundingCheck {
                memory_id: operation.memory_ids[0].clone(),
                expected_revision: 1,
                anchors: Vec::new(),
                grounding: Grounding::Unavailable,
                reasons: vec!["supporting_evidence_changed".into()],
            }],
        )
        .await
        .unwrap();
    let result = service.read(&context, read("scoring")).await.unwrap();
    assert!(result.memories.is_empty());
    assert_eq!(result.needs_verification.len(), 1);
    let mut query = read("scoring");
    query.include_needs_verification = false;
    assert!(
        service
            .read(&context, query)
            .await
            .unwrap()
            .memories
            .is_empty()
    );
}
#[tokio::test]
async fn corrections_require_revision_and_history_preserves_previous_statement() {
    let service = service().await;
    let context = context("project", None);
    let operation = active(&service, &context, "scoring old constraint").await;
    let id = &operation.memory_ids[0];
    let at = chrono::Utc::now();
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let selected: UpdateRequest =
        serde_json::from_value(serde_json::json!({"query":"scoring"})).unwrap();
    assert_eq!(
        service.update(&context, selected).await.unwrap()["status"],
        "needs_selection"
    );
    let update:UpdateRequest=serde_json::from_value(serde_json::json!({"memory_id":id,"expected_revision":1,"statement":"scoring new constraint"})).unwrap();
    service.update(&context, update.clone()).await.unwrap();
    assert!(service.update(&context, update).await.is_err());
    let mut historical = read("scoring");
    historical.as_of = Some(at);
    assert_eq!(
        service.read(&context, historical).await.unwrap().memories[0]
            .memory
            .statement,
        "scoring old constraint"
    );
}
#[tokio::test]
async fn feedback_does_not_confirm_or_extend_ttl() {
    let service = service().await;
    let context = context("project", None);
    let mut request = write("scoring");
    request.ttl_seconds = Some(3600);
    let operation = service.write(&context, request).await.unwrap();
    service.process_next().await.unwrap();
    let id = service
        .status(&context, &operation.id)
        .await
        .unwrap()
        .memory_ids[0]
        .clone();
    let before = service.store.lock().await.state.claims[&id].clone();
    let update:UpdateRequest=serde_json::from_value(serde_json::json!({"memory_id":id,"expected_revision":1,"useful":true,"feedback_id":"feedback-1"})).unwrap();
    service.update(&context, update.clone()).await.unwrap();
    service.update(&context, update).await.unwrap();
    let after = service.store.lock().await.state.claims[&id].clone();
    assert_eq!(after.expires_at, before.expires_at);
    assert_eq!(after.confirmed_at, None);
    assert_eq!(after.usefulness, 1);
}
#[tokio::test]
async fn forgetting_purges_sources_and_keeps_sibling_claims() {
    let service = service().await;
    let context = context("project", None);
    let operation = active(
        &service,
        &context,
        "scoring secret|cat unrelated preference",
    )
    .await;
    let id = service
        .store
        .lock()
        .await
        .state
        .claims
        .values()
        .find(|v| v.statement.contains("secret"))
        .unwrap()
        .id
        .clone();
    let delete: DeleteRequest =
        serde_json::from_value(serde_json::json!({"memory_id":id,"expected_revision":1})).unwrap();
    service.delete(&context, delete).await.unwrap();
    let state = service.store.lock().await;
    let payload = serde_json::to_string(&state.state).unwrap();
    assert!(!payload.contains("scoring secret"));
    assert!(payload.contains("cat unrelated preference"));
    assert!(state.state.observations.is_empty());
    assert_eq!(
        state.state.operations[&operation.id].state,
        JobState::Cancelled
    );
}
#[tokio::test]
async fn embedding_identity_mismatch_never_becomes_lexical_only() {
    let service = service().await;
    let context = context("project", None);
    active(&service, &context, "scoring").await;
    let other = MemoryService {
        embedder: Arc::new(Embeddings("vectors-v2")),
        ..service.clone()
    };
    assert!(
        other
            .read(&context, read("scoring"))
            .await
            .unwrap_err()
            .to_string()
            .contains("identity")
    );
    assert!(other.write(&context, write("scoring new")).await.is_err());
}
#[tokio::test]
async fn pending_jobs_survive_store_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("memory-db");
    let service = MemoryService::new(
        Store::open(Some(&path)).await.unwrap(),
        Arc::new(Embeddings("vectors-v1")),
        Some(Arc::new(Classification)),
        None,
    );
    let context = context("project", None);
    let operation = service
        .write(&context, write("scoring persisted"))
        .await
        .unwrap();
    drop(service);
    // SurrealKV closes its background flush tasks asynchronously after the last handle drops.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let service = MemoryService::new(
        Store::open(Some(&path)).await.unwrap(),
        Arc::new(Embeddings("vectors-v1")),
        Some(Arc::new(Classification)),
        None,
    );
    service.process_next().await.unwrap();
    assert_eq!(
        service.status(&context, &operation.id).await.unwrap().state,
        JobState::Complete
    );
    assert_eq!(
        service
            .read(&context, read("scoring"))
            .await
            .unwrap()
            .memories
            .len(),
        1
    );
}
#[tokio::test]
async fn asynchronous_acceptance_exposes_pending_embedding() {
    let service = service().await;
    let context = context("project", Some("s"));
    let mut request = write("scoring");
    request.asynchronous = true;
    let operation = service.write(&context, request).await.unwrap();
    assert!(!operation.embedding_ready);
    assert_eq!(operation.stage, "embedding_pending");
    let pending = service.read(&context, read("scoring")).await.unwrap();
    assert!(
        pending.memories.is_empty(),
        "pending embeddings must not become lexical-only recall"
    );
    assert_eq!(pending.status, "partial");
    service.process_next().await.unwrap();
    assert!(
        service
            .status(&context, &operation.id)
            .await
            .unwrap()
            .embedding_ready
    );
}

#[tokio::test]
async fn user_scope_crosses_projects_but_session_scope_never_does() {
    let service = service().await;
    let origin = context("project-a", Some("a"));
    let mut request = write("scoring personal preference");
    request.scope = Scope::User;
    let operation = service.write(&origin, request).await.unwrap();
    service.process_next().await.unwrap();
    assert_eq!(
        service
            .status(&context("project-b", None), &operation.id)
            .await
            .unwrap()
            .state,
        JobState::Complete
    );
    assert_eq!(
        service
            .read(&context("project-b", None), read("scoring"))
            .await
            .unwrap()
            .memories
            .len(),
        1
    );
    let mut request = write("scoring task state");
    request.scope = Scope::Session;
    assert!(
        service
            .write(&context("project-a", None), request.clone())
            .await
            .is_err()
    );
    let operation = service.write(&origin, request).await.unwrap();
    service.process_next().await.unwrap();
    let id = service
        .status(&origin, &operation.id)
        .await
        .unwrap()
        .memory_ids[0]
        .clone();
    let update = serde_json::from_value(
        serde_json::json!({"memory_id":id,"expected_revision":1,"complete":true}),
    )
    .unwrap();
    service.update(&origin, update).await.unwrap();
    let mut query = read("scoring task");
    query.scope = Some(Scope::Session);
    assert!(
        service
            .read(&origin, query.clone())
            .await
            .unwrap()
            .memories
            .is_empty()
    );
    query.include_archived = true;
    assert_eq!(
        service
            .read(&origin, query.clone())
            .await
            .unwrap()
            .memories
            .len(),
        1
    );
    assert!(
        service
            .read(&context("project-a", Some("b")), query)
            .await
            .unwrap()
            .memories
            .is_empty()
    );
}

struct FailingEmbeddings;
#[async_trait]
impl Embedder for FailingEmbeddings {
    fn identity(&self) -> Result<String> {
        Ok("vectors-broken".into())
    }
    async fn document(&self, _: &str) -> Result<Vec<Vec<f32>>> {
        anyhow::bail!("offline failure")
    }
    async fn query(&self, _: &str) -> Result<Vec<f32>> {
        anyhow::bail!("offline failure")
    }
}
#[tokio::test]
async fn reembedding_is_atomic_and_updates_historical_vectors() {
    let service = service().await;
    let context = context("project", None);
    let operation = active(&service, &context, "scoring old").await;
    let update = serde_json::from_value(serde_json::json!({"memory_id":operation.memory_ids[0],"expected_revision":1,"statement":"scoring new"})).unwrap();
    service.update(&context, update).await.unwrap();
    assert!(service.reembed(Arc::new(FailingEmbeddings)).await.is_err());
    assert_eq!(
        service
            .store
            .lock()
            .await
            .state
            .embedding_identity
            .as_deref(),
        Some("vectors-v1")
    );
    service
        .reembed(Arc::new(Embeddings("vectors-v2")))
        .await
        .unwrap();
    let store = service.store.lock().await;
    assert_eq!(
        store.state.embedding_identity.as_deref(),
        Some("vectors-v2")
    );
    assert!(
        store
            .state
            .revisions
            .iter()
            .all(|v| v.claim.embedding_identity == "vectors-v2")
    );
}

struct BlockedClassifier {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait]
impl Classifier for BlockedClassifier {
    fn identity(&self) -> String {
        "blocked-v1".into()
    }
    async fn extract(&self, observation: &Observation) -> Result<Vec<Proposal>> {
        self.entered.notify_one();
        self.release.notified().await;
        Classification.extract(observation).await
    }
    async fn reconcile(
        &self,
        observation: &Observation,
        claims: Vec<Proposal>,
        candidates: Vec<Claim>,
    ) -> Result<Vec<Proposal>> {
        Classification
            .reconcile(observation, claims, candidates)
            .await
    }
}
#[tokio::test]
async fn forgetting_during_inflight_classification_cannot_resurrect_content() {
    let base = service().await;
    let classifier = Arc::new(BlockedClassifier {
        entered: Default::default(),
        release: Default::default(),
    });
    let service = MemoryService {
        classifier: Some(classifier.clone()),
        ..base
    };
    let context = context("project", Some("s"));
    let operation = service
        .write(&context, write("scoring secret"))
        .await
        .unwrap();
    let worker = service.clone();
    let processing = tokio::spawn(async move { worker.process_next().await });
    classifier.entered.notified().await;
    service
        .delete(
            &context,
            serde_json::from_value(
                serde_json::json!({"memory_id":operation.memory_ids[0],"expected_revision":1}),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    classifier.release.notify_one();
    processing.await.unwrap().unwrap();
    let store = service.store.lock().await;
    assert!(store.state.claims.is_empty());
    assert!(
        !serde_json::to_string(&store.state)
            .unwrap()
            .contains("scoring secret")
    );
    assert_eq!(
        store.state.operations[&operation.id].state,
        JobState::Cancelled
    );
}

#[tokio::test]
async fn blocked_model_job_does_not_starve_compatible_job() {
    let base = service().await;
    let classifier = Arc::new(BlockedClassifier {
        entered: Default::default(),
        release: Default::default(),
    });
    let other = MemoryService {
        classifier: Some(classifier),
        ..base.clone()
    };
    let context = context("project", None);
    let blocked = other
        .write(&context, write("cat deferred old model"))
        .await
        .unwrap();
    let available = base
        .write(&context, write("scoring current model"))
        .await
        .unwrap();
    base.process_next().await.unwrap();
    assert_eq!(
        base.status(&context, &blocked.id).await.unwrap().state,
        JobState::ConfigRequired
    );
    assert_eq!(
        base.status(&context, &available.id).await.unwrap().state,
        JobState::Complete
    );
}

struct FailingReranker;
#[async_trait]
impl codegraph_vector::reranking::Reranker for FailingReranker {
    fn model_name(&self) -> &str {
        "offline"
    }
    fn provider_name(&self) -> &str {
        "mock"
    }
    async fn rerank(
        &self,
        _: &str,
        _: Vec<codegraph_vector::reranking::RerankDocument>,
        _: usize,
    ) -> Result<Vec<codegraph_vector::reranking::RerankResult>> {
        anyhow::bail!("offline failure")
    }
}
#[tokio::test]
async fn reranker_failure_keeps_semantic_results_and_declares_partial_status() {
    let base = service().await;
    let service = MemoryService {
        reranker: Some(Arc::new(FailingReranker)),
        ..base
    };
    let context = context("project", None);
    active(&service, &context, "scoring").await;
    let result = service.read(&context, read("scoring")).await.unwrap();
    assert_eq!(result.status, "partial");
    assert_eq!(result.memories.len(), 1);
    assert!(result.warnings.iter().any(|v| v.contains("reranker")));
}

#[tokio::test]
async fn historical_conflicts_and_confirmation_keep_revision_boundaries() {
    let service = service().await;
    let context = context("project", None);
    active(&service, &context, "parallel scoring is unsafe").await;
    let before = chrono::Utc::now();
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    active(&service, &context, "parallel scoring is safe").await;
    let mut query = read("parallel scoring");
    query.as_of = Some(before);
    let result = service.read(&context, query).await.unwrap();
    assert_eq!(result.memories.len(), 1);
    assert!(result.needs_verification.is_empty());
    assert_eq!(
        result.memories[0].memory.evidence_state,
        EvidenceState::Reported
    );
}

#[tokio::test]
async fn mandatory_constraints_overflow_explicitly_and_context_counts_wrappers() {
    let service = service().await;
    let context = context("project", None);
    active(&service, &context, "scoring rule").await;
    let mut entries = service
        .read(&context, read("scoring"))
        .await
        .unwrap()
        .memories;
    entries[0].memory.tier = Tier::Core;
    assert!(
        codegraph_memory::context::pack(
            entries.clone(),
            10,
            500,
            &codegraph_memory::context::ConservativeCounter
        )
        .is_err()
    );
    let mut packed = codegraph_memory::context::pack(
        entries,
        10,
        20_000,
        &codegraph_memory::context::ConservativeCounter,
    )
    .unwrap();
    let reference = packed.memories[0].reference.clone();
    packed.record_citations(&format!("[{reference}] [memory:invented@1]"));
    assert_eq!(packed.cited_memory_refs, vec![reference]);
    assert!(serde_json::to_string(&packed).unwrap().len() <= packed.estimated_tokens);
    let unavailable = codegraph_memory::context::pack_with_metadata(
        vec![],
        10,
        3000,
        &codegraph_memory::context::ConservativeCounter,
        MemoryContext::unavailable("provider unavailable"),
    )
    .unwrap();
    assert_eq!(unavailable.status, "unavailable");
}

#[tokio::test]
async fn applicability_conditions_filter_recall_and_reconciliation() {
    let service = service().await;
    let context = context("project", None);
    for branch in ["main", "experimental"] {
        let mut request = write("scoring branch-specific decision");
        request.applicability.insert("branch".into(), branch.into());
        service.write(&context, request).await.unwrap();
        service.process_next().await.unwrap();
    }
    assert_eq!(
        service.store.lock().await.state.claims.len(),
        2,
        "different conditions cannot authorize equivalence"
    );
    assert!(
        service
            .read(&context, read("scoring"))
            .await
            .unwrap()
            .memories
            .is_empty()
    );
    let mut query = read("scoring");
    query.applicability.insert("branch".into(), "main".into());
    let result = service.read(&context, query).await.unwrap();
    assert_eq!(result.memories.len(), 1);
    assert_eq!(result.memories[0].memory.applicability["branch"], "main");
    assert_eq!(
        serde_json::from_value::<ReadRequest>(
            serde_json::json!({"query":"scoring","scope":"both"})
        )
        .unwrap()
        .scope,
        None
    );
}

#[tokio::test]
async fn equivalent_observation_adds_anchors_without_verifying_the_claim() {
    let service = service().await;
    let context = context("project", None);
    let initial = active(&service, &context, "scoring rule").await;
    let mut request = write("scoring rule");
    request.anchors.push(Anchor {
        node_id: "nodes:score".into(),
        graph_project_id: "graph".into(),
        path: "src/lib.rs".into(),
        symbol: "score".into(),
        supporting: true,
        historical_snippet: "fn score() {}".into(),
        span_hash: "fixture-hash".into(),
        ..Default::default()
    });
    let operation = service.write(&context, request).await.unwrap();
    service.process_next().await.unwrap();
    let completed = service.status(&context, &operation.id).await.unwrap();
    assert_eq!(completed.memory_ids, initial.memory_ids);
    let result = service.read(&context, read("scoring")).await.unwrap();
    let memory = &result.memories[0].memory;
    assert_eq!(memory.revision, 2);
    assert_eq!(memory.anchors.len(), 1);
    assert_eq!(memory.grounding, Grounding::Anchored);
    assert_eq!(memory.evidence_state, EvidenceState::Reported);
    assert_eq!(memory.confirmed_at, None);
}
