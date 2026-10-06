// ABOUTME: Shared memory request, evidence, lifecycle, and background operation contracts.
// ABOUTME: Access scope, authority, grounding, and evidence are independent dimensions.
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

macro_rules! enumeration {
    ($name:ident, $default:ident, $($variant:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { #[default] $default, $($variant),+ }
    };
}
enumeration!(Scope, Project, Session, User);
enumeration!(
    Kind,
    Unclassified,
    Fact,
    Decision,
    Preference,
    Procedure,
    Episode,
    WorkingState
);
enumeration!(Tier, Durable, Working, Core, Archive);
enumeration!(
    Lifecycle,
    Provisional,
    Active,
    Superseded,
    Archived,
    Retracted
);
enumeration!(EvidenceState, Reported, Verified, Disputed);
enumeration!(Grounding, NotApplicable, Anchored, Unresolved, Unavailable);
enumeration!(
    JobState,
    Pending,
    Running,
    Complete,
    Failed,
    ConfigRequired,
    Cancelled
);
enumeration!(
    RelationshipKind,
    Unrelated,
    Equivalent,
    Complementary,
    Contradictory,
    Correction
);

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClientContext {
    pub owner_id: String,
    pub project_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub context_epoch: Option<String>,
    #[serde(default)]
    pub applicability: BTreeMap<String, String>,
}
impl ClientContext {
    pub fn authorizes(&self, memory: &Claim) -> bool {
        memory.owner_id == self.owner_id
            && match memory.scope {
                Scope::User => true,
                Scope::Project => memory.project_id == self.project_id,
                Scope::Session => {
                    memory.project_id == self.project_id
                        && self.session_id.is_some()
                        && memory.session_id == self.session_id
                }
            }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Evidence {
    pub uri: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub verified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Anchor {
    pub node_id: String,
    pub graph_project_id: String,
    pub path: String,
    pub symbol: String,
    pub start_line: u32,
    pub end_line: u32,
    pub file_hash: String,
    pub span_hash: String,
    #[serde(default)]
    pub input_fingerprint: Option<String>,
    #[serde(default)]
    pub historical_snippet: String,
    #[serde(default)]
    pub supporting: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriteRequest {
    pub statement: String,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub applicability: BTreeMap<String, String>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
    #[serde(default)]
    pub asynchronous: bool,
    #[serde(default)]
    pub anchors: Vec<Anchor>,
    /// Explicit supplied locators make failed grounding distinguishable from non-code content.
    #[serde(default)]
    pub code_related: bool,
    #[serde(default)]
    pub authorized_constraint: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadRequest {
    pub query: String,
    #[serde(default)]
    pub applicability: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "read_scope")]
    #[schemars(with = "Option<ReadScope>")]
    pub scope: Option<Scope>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub kinds: Vec<Kind>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_budget")]
    pub token_budget: usize,
    #[serde(default = "yes")]
    pub include_needs_verification: bool,
    #[serde(default)]
    pub include_stale: bool,
    #[serde(default)]
    pub include_expired: bool,
    #[serde(default)]
    pub include_archived: bool,
    #[serde(default)]
    pub include_provisional: bool,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub node_ids: Vec<String>,
}
fn default_limit() -> usize {
    10
}
fn default_budget() -> usize {
    3000
}
fn yes() -> bool {
    true
}
impl ReadRequest {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            applicability: Default::default(),
            scope: None,
            symbol: None,
            node_id: None,
            kinds: Vec::new(),
            limit: 10,
            token_budget: 3000,
            include_needs_verification: true,
            include_stale: false,
            include_expired: false,
            include_archived: false,
            include_provisional: false,
            as_of: None,
            since: None,
            operation_id: None,
            node_ids: Vec::new(),
        }
    }
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadScope {
    Both,
    Project,
    Session,
    User,
}
fn read_scope<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Scope>, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    match value.as_deref() {
        None | Some("both") => Ok(None),
        Some("project") => Ok(Some(Scope::Project)),
        Some("session") => Ok(Some(Scope::Session)),
        Some("user") => Ok(Some(Scope::User)),
        _ => Err(serde::de::Error::custom(
            "Read scope must be both, project, session, or user",
        )),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateRequest {
    #[serde(default)]
    pub scope: Option<Scope>,
    #[serde(default)]
    pub memory_id: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub statement: Option<String>,
    #[serde(default)]
    pub confirm: bool,
    /// Declared reusable procedure; confirmation alone never implies global applicability.
    #[serde(default)]
    pub reusable: bool,
    #[serde(default)]
    pub complete: bool,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub useful: Option<bool>,
    #[serde(default)]
    pub feedback_id: Option<String>,
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteRequest {
    #[serde(default)]
    pub scope: Option<Scope>,
    #[serde(default)]
    pub memory_id: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Claim {
    pub id: String,
    pub revision: u64,
    pub statement: String,
    pub owner_id: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub scope: Scope,
    pub kind: Kind,
    pub tier: Tier,
    pub lifecycle: Lifecycle,
    pub evidence_state: EvidenceState,
    pub grounding: Grounding,
    pub evidence: Vec<Evidence>,
    pub anchors: Vec<Anchor>,
    pub observation_ids: Vec<String>,
    pub applicability: BTreeMap<String, String>,
    pub recorded_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub valid_from: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub review_after: Option<DateTime<Utc>>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub review_reasons: Vec<String>,
    pub conflicts: Vec<String>,
    pub classification_identity: Option<String>,
    pub embedding_identity: String,
    pub vectors: Vec<Vec<f32>>,
    pub usefulness: i64,
    pub feedback_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub context: ClientContext,
    pub request: WriteRequest,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Operation {
    pub id: String,
    pub owner_id: String,
    pub project_id: String,
    pub scope: Scope,
    pub session_id: Option<String>,
    pub observation_id: String,
    pub memory_ids: Vec<String>,
    pub state: JobState,
    pub stage: String,
    pub embedding_ready: bool,
    pub attempts: u32,
    pub lease_until: Option<DateTime<Utc>>,
    pub retry_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub model_identity: String,
    #[serde(default)]
    pub correction_target: Option<(String, u64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Revision {
    pub claim: Claim,
    pub changed_at: DateTime<Utc>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Relationship {
    pub source: String,
    pub target: String,
    pub kind: RelationshipKind,
    #[serde(default)]
    pub source_revision: Option<u64>,
    #[serde(default)]
    pub target_revision: Option<u64>,
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub statement: String,
    pub kind: Kind,
    #[serde(default)]
    pub working: bool,
    #[serde(default)]
    pub core_candidate: bool,
    #[serde(default)]
    pub code_related: bool,
    #[serde(default)]
    pub relationships: Vec<ProposedRelationship>,
    /// References into supplied evidence; new references are rejected.
    #[serde(default)]
    pub evidence_indices: Vec<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposedRelationship {
    pub memory_id: String,
    pub expected_revision: u64,
    pub kind: RelationshipKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CodeContext {
    #[serde(default)]
    pub truncated: bool,
    pub nodes: Vec<serde_json::Value>,
    pub edges: Vec<serde_json::Value>,
    pub paths: Vec<serde_json::Value>,
    pub snippets: Vec<serde_json::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundingCheck {
    pub memory_id: String,
    pub expected_revision: u64,
    pub anchors: Vec<Anchor>,
    pub grounding: Grounding,
    pub reasons: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryEntry {
    pub memory: Claim,
    pub reference: String,
    pub reasons: Vec<String>,
    pub code_context: CodeContext,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MemoryContext {
    pub status: String,
    #[serde(default)]
    pub limit: usize,
    #[serde(default)]
    pub token_budget: usize,
    pub memories: Vec<MemoryEntry>,
    pub needs_verification: Vec<MemoryEntry>,
    pub retrieved_memory_refs: Vec<String>,
    pub cited_memory_refs: Vec<String>,
    pub warnings: Vec<String>,
    pub truncated: usize,
    pub estimated_tokens: usize,
    pub operation: Option<Operation>,
}
impl MemoryContext {
    pub fn disabled() -> Self {
        Self {
            status: "disabled".into(),
            ..Self::default()
        }
    }
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: "unavailable".into(),
            warnings: vec![reason.into()],
            ..Self::default()
        }
    }
    pub fn record_citations(&mut self, answer: &str) {
        self.cited_memory_refs = self
            .retrieved_memory_refs
            .iter()
            .filter(|reference| answer.contains(&format!("[{}]", reference)))
            .cloned()
            .collect();
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    #[serde(default)]
    pub generation: u64,
    pub claims: BTreeMap<String, Claim>,
    pub observations: BTreeMap<String, Observation>,
    pub operations: BTreeMap<String, Operation>,
    pub revisions: Vec<Revision>,
    pub relationships: Vec<Relationship>,
    /// Scoped idempotency digest -> content digest, operation id.
    pub idempotency: BTreeMap<String, (String, String)>,
    pub tombstones: BTreeMap<String, u64>,
    pub embedding_identity: Option<String>,
}
