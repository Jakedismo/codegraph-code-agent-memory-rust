-- Durable, typed memory records. Code indexes remain independently rebuildable.
-- HNSW partitions are instantiated from agent_memory_vectors.surql for the pinned identity.
DEFINE TABLE IF NOT EXISTS memory_meta SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS schema_version ON memory_meta TYPE int;
DEFINE FIELD IF NOT EXISTS generation ON memory_meta TYPE int;
DEFINE FIELD IF NOT EXISTS embedding_identity ON memory_meta TYPE option<string>;

DEFINE TABLE IF NOT EXISTS memory_claim SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS memory_id ON memory_claim TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON memory_claim TYPE int;
DEFINE FIELD IF NOT EXISTS statement ON memory_claim TYPE string;
DEFINE FIELD IF NOT EXISTS owner_id ON memory_claim TYPE string;
DEFINE FIELD IF NOT EXISTS project_id ON memory_claim TYPE string;
DEFINE FIELD IF NOT EXISTS session_id ON memory_claim TYPE option<string>;
DEFINE FIELD IF NOT EXISTS scope ON memory_claim TYPE 'project' | 'session' | 'user';
DEFINE FIELD IF NOT EXISTS kind ON memory_claim TYPE 'unclassified' | 'fact' | 'decision' | 'preference' | 'procedure' | 'episode' | 'working_state';
DEFINE FIELD IF NOT EXISTS tier ON memory_claim TYPE 'working' | 'durable' | 'core' | 'archive';
DEFINE FIELD IF NOT EXISTS lifecycle ON memory_claim TYPE 'provisional' | 'active' | 'superseded' | 'archived' | 'retracted';
DEFINE FIELD IF NOT EXISTS evidence_state ON memory_claim TYPE 'reported' | 'verified' | 'disputed';
DEFINE FIELD IF NOT EXISTS grounding ON memory_claim TYPE 'not_applicable' | 'anchored' | 'unresolved' | 'unavailable';
DEFINE FIELD IF NOT EXISTS evidence ON memory_claim TYPE array<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS anchors ON memory_claim TYPE array<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS observation_ids ON memory_claim TYPE array<string>;
DEFINE FIELD IF NOT EXISTS applicability ON memory_claim TYPE object FLEXIBLE;
DEFINE FIELD IF NOT EXISTS recorded_at ON memory_claim TYPE datetime;
DEFINE FIELD IF NOT EXISTS observed_at ON memory_claim TYPE datetime;
DEFINE FIELD IF NOT EXISTS valid_from ON memory_claim TYPE datetime;
DEFINE FIELD IF NOT EXISTS valid_until ON memory_claim TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS expires_at ON memory_claim TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS review_after ON memory_claim TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS confirmed_at ON memory_claim TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS review_reasons ON memory_claim TYPE array<string>;
DEFINE FIELD IF NOT EXISTS conflicts ON memory_claim TYPE array<string>;
DEFINE FIELD IF NOT EXISTS classification_identity ON memory_claim TYPE option<string>;
DEFINE FIELD IF NOT EXISTS embedding_identity ON memory_claim TYPE string;
DEFINE FIELD IF NOT EXISTS vectors ON memory_claim TYPE array<array<float>>;
DEFINE FIELD IF NOT EXISTS usefulness ON memory_claim TYPE int;
DEFINE FIELD IF NOT EXISTS feedback_ids ON memory_claim TYPE array<string>;

DEFINE TABLE IF NOT EXISTS memory_observation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS observation_id ON memory_observation TYPE string;
DEFINE FIELD IF NOT EXISTS context ON memory_observation TYPE object FLEXIBLE;
DEFINE FIELD IF NOT EXISTS request ON memory_observation TYPE object FLEXIBLE;
DEFINE FIELD IF NOT EXISTS recorded_at ON memory_observation TYPE datetime;

DEFINE TABLE IF NOT EXISTS memory_revision SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS memory_id ON memory_revision TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON memory_revision TYPE int;
DEFINE FIELD IF NOT EXISTS statement ON memory_revision TYPE string;
DEFINE FIELD IF NOT EXISTS claim ON memory_revision TYPE object FLEXIBLE;
DEFINE FIELD IF NOT EXISTS changed_at ON memory_revision TYPE datetime;
DEFINE FIELD IF NOT EXISTS reason ON memory_revision TYPE string;

DEFINE TABLE IF NOT EXISTS memory_job SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS operation_id ON memory_job TYPE string;
DEFINE FIELD IF NOT EXISTS owner_id ON memory_job TYPE string;
DEFINE FIELD IF NOT EXISTS project_id ON memory_job TYPE string;
DEFINE FIELD IF NOT EXISTS scope ON memory_job TYPE 'project' | 'session' | 'user';
DEFINE FIELD IF NOT EXISTS session_id ON memory_job TYPE option<string>;
DEFINE FIELD IF NOT EXISTS observation_id ON memory_job TYPE string;
DEFINE FIELD IF NOT EXISTS memory_ids ON memory_job TYPE array<string>;
DEFINE FIELD IF NOT EXISTS state ON memory_job TYPE 'pending' | 'running' | 'complete' | 'failed' | 'config_required' | 'cancelled';
DEFINE FIELD IF NOT EXISTS stage ON memory_job TYPE string;
DEFINE FIELD IF NOT EXISTS embedding_ready ON memory_job TYPE bool;
DEFINE FIELD IF NOT EXISTS attempts ON memory_job TYPE int;
DEFINE FIELD IF NOT EXISTS lease_until ON memory_job TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS retry_at ON memory_job TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS error ON memory_job TYPE option<string>;
DEFINE FIELD IF NOT EXISTS model_identity ON memory_job TYPE string;
DEFINE FIELD IF NOT EXISTS correction_target ON memory_job TYPE option<array>;

DEFINE TABLE IF NOT EXISTS memory_idempotency SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS digest ON memory_idempotency TYPE string;
DEFINE FIELD IF NOT EXISTS content_digest ON memory_idempotency TYPE string;
DEFINE FIELD IF NOT EXISTS operation_id ON memory_idempotency TYPE string;

DEFINE TABLE IF NOT EXISTS memory_tombstone SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS memory_id ON memory_tombstone TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON memory_tombstone TYPE int;

DEFINE TABLE IF NOT EXISTS memory_evidence SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS claim ON memory_evidence TYPE record<memory_claim>;
DEFINE FIELD IF NOT EXISTS revision ON memory_evidence TYPE int;
DEFINE FIELD IF NOT EXISTS uri ON memory_evidence TYPE string;
DEFINE FIELD IF NOT EXISTS description ON memory_evidence TYPE string;
DEFINE FIELD IF NOT EXISTS fingerprint ON memory_evidence TYPE option<string>;
DEFINE FIELD IF NOT EXISTS verified_at ON memory_evidence TYPE option<datetime>;

DEFINE TABLE IF NOT EXISTS memory_code_locator SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS node_id ON memory_code_locator TYPE string;
DEFINE FIELD IF NOT EXISTS graph_project_id ON memory_code_locator TYPE string;
DEFINE FIELD IF NOT EXISTS path ON memory_code_locator TYPE string;
DEFINE FIELD IF NOT EXISTS symbol ON memory_code_locator TYPE string;
DEFINE FIELD IF NOT EXISTS start_line ON memory_code_locator TYPE int;
DEFINE FIELD IF NOT EXISTS end_line ON memory_code_locator TYPE int;
DEFINE FIELD IF NOT EXISTS file_hash ON memory_code_locator TYPE string;
DEFINE FIELD IF NOT EXISTS span_hash ON memory_code_locator TYPE string;
DEFINE FIELD IF NOT EXISTS input_fingerprint ON memory_code_locator TYPE option<string>;
DEFINE FIELD IF NOT EXISTS historical_snippet ON memory_code_locator TYPE string;
DEFINE FIELD IF NOT EXISTS supporting ON memory_code_locator TYPE bool;

DEFINE TABLE IF NOT EXISTS memory_relationship TYPE RELATION IN memory_claim OUT memory_claim SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS source ON memory_relationship TYPE string;
DEFINE FIELD IF NOT EXISTS target ON memory_relationship TYPE string;
DEFINE FIELD IF NOT EXISTS kind ON memory_relationship TYPE 'unrelated' | 'equivalent' | 'complementary' | 'contradictory' | 'correction';
DEFINE FIELD IF NOT EXISTS source_revision ON memory_relationship TYPE option<int>;
DEFINE FIELD IF NOT EXISTS target_revision ON memory_relationship TYPE option<int>;
DEFINE FIELD IF NOT EXISTS operation_id ON memory_relationship TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON memory_relationship TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS evidence ON memory_relationship TYPE array<object> FLEXIBLE;
DEFINE TABLE IF NOT EXISTS memory_derived_from TYPE RELATION IN memory_claim OUT memory_observation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS revision ON memory_derived_from TYPE int;
DEFINE TABLE IF NOT EXISTS memory_link TYPE RELATION IN memory_claim OUT memory_code_locator SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS revision ON memory_link TYPE int;
DEFINE FIELD IF NOT EXISTS role ON memory_link TYPE 'evidence' | 'related';

DEFINE ANALYZER IF NOT EXISTS memory_prose TOKENIZERS blank,class,camel,punct FILTERS lowercase,ascii;
DEFINE INDEX IF NOT EXISTS memory_claim_text_v2 ON memory_claim FIELDS statement FULLTEXT ANALYZER memory_prose BM25;
DEFINE INDEX IF NOT EXISTS memory_claim_access ON memory_claim FIELDS owner_id,project_id,scope,session_id,lifecycle;
DEFINE INDEX IF NOT EXISTS memory_claim_review ON memory_claim FIELDS owner_id,project_id,review_after;
DEFINE INDEX IF NOT EXISTS memory_claim_expiry ON memory_claim FIELDS owner_id,project_id,expires_at;
DEFINE INDEX IF NOT EXISTS memory_revision_identity ON memory_revision FIELDS memory_id,revision UNIQUE;
DEFINE INDEX IF NOT EXISTS memory_job_queue ON memory_job FIELDS state,retry_at,lease_until;
DEFINE INDEX IF NOT EXISTS memory_job_access ON memory_job FIELDS owner_id,project_id,scope,session_id;
DEFINE INDEX IF NOT EXISTS memory_link_endpoints ON memory_link FIELDS in,out,revision;
DEFINE INDEX IF NOT EXISTS memory_locator_target ON memory_code_locator FIELDS graph_project_id,node_id;
DEFINE INDEX IF NOT EXISTS memory_relationship_endpoints ON memory_relationship FIELDS in,out,kind;
DEFINE INDEX IF NOT EXISTS memory_derivation_endpoints ON memory_derived_from FIELDS in,out;
DEFINE INDEX IF NOT EXISTS memory_evidence_claim ON memory_evidence FIELDS claim,revision;
