-- Rebuildable, content-free project memory references in the code database.
-- Durable claims, source text, evidence, user/session memories remain in the memory store.
DEFINE TABLE IF NOT EXISTS memory_projection SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS source_id ON memory_projection TYPE string;
DEFINE FIELD IF NOT EXISTS project_id ON memory_projection TYPE string;
DEFINE FIELD IF NOT EXISTS memory_id ON memory_projection TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON memory_projection TYPE int;
DEFINE INDEX IF NOT EXISTS memory_projection_source ON memory_projection FIELDS source_id,project_id,memory_id UNIQUE;
DEFINE TABLE IF NOT EXISTS memory_code_anchor TYPE RELATION IN memory_projection OUT nodes SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS source_id ON memory_code_anchor TYPE string;
DEFINE FIELD IF NOT EXISTS project_id ON memory_code_anchor TYPE string;
DEFINE FIELD IF NOT EXISTS role ON memory_code_anchor TYPE 'evidence' | 'related';
DEFINE INDEX IF NOT EXISTS memory_code_anchor_target ON memory_code_anchor FIELDS source_id,project_id,out;
DEFINE TABLE IF NOT EXISTS memory_projection_cursor SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS project_id ON memory_projection_cursor TYPE string;
DEFINE FIELD IF NOT EXISTS source_id ON memory_projection_cursor TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON memory_projection_cursor TYPE int;
DEFINE FIELD IF NOT EXISTS memory_id ON memory_projection_cursor TYPE string;
DEFINE FIELD IF NOT EXISTS code_input_fingerprint ON memory_projection_cursor TYPE option<string>;
