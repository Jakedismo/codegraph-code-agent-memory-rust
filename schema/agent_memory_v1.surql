-- Legacy v1 bootstrap retained for migration fixtures; not the current runtime schema.
-- See agent_memory_v2.surql, agent_memory_vectors.surql, and agent_memory_queries.surql.
DEFINE TABLE IF NOT EXISTS memory_state SCHEMALESS;
DEFINE TABLE IF NOT EXISTS memory_claim SCHEMALESS;
DEFINE ANALYZER IF NOT EXISTS memory_text TOKENIZERS blank,class FILTERS lowercase,ascii;
DEFINE INDEX IF NOT EXISTS memory_claim_text ON memory_claim FIELDS statement FULLTEXT ANALYZER memory_text BM25;
DEFINE INDEX IF NOT EXISTS memory_claim_access ON memory_claim FIELDS owner_id,project_id,scope,session_id;
DEFINE TABLE IF NOT EXISTS memory_observation SCHEMALESS;
DEFINE TABLE IF NOT EXISTS memory_revision SCHEMALESS;
DEFINE TABLE IF NOT EXISTS memory_relationship SCHEMALESS;
DEFINE TABLE IF NOT EXISTS memory_job SCHEMALESS;
DEFINE TABLE IF NOT EXISTS memory_tombstone SCHEMALESS;
-- Vector partitions are created for a pinned embedding identity and dimension at runtime.
