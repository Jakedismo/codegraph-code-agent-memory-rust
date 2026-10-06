-- Rendered for each pinned embedding identity; identifiers/dimension are generated, never user SQL.
DEFINE TABLE IF NOT EXISTS __TABLE__ SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS memory_id ON __TABLE__ TYPE string;
DEFINE FIELD IF NOT EXISTS claim ON __TABLE__ TYPE record<memory_claim>;
DEFINE FIELD IF NOT EXISTS chunk ON __TABLE__ TYPE int;
DEFINE FIELD IF NOT EXISTS vector ON __TABLE__ TYPE array<float>;
DEFINE INDEX IF NOT EXISTS memory_vector_access ON __TABLE__ FIELDS memory_id;
DEFINE INDEX IF NOT EXISTS memory_vector_hnsw ON __TABLE__ FIELDS vector HNSW DIMENSION __DIMENSION__ DIST COSINE TYPE F32;
