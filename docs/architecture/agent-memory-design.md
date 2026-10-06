# Agent context and memory: design

Status: initial semantic-memory implementation available in October 2026. The independent
memory crate, durable owner, semantic CLI/MCP operations, background classification,
scope/lifecycle policy, optional code grounding, and opt-in agent context assembly are
implemented. See [usage and current implementation boundaries](../AGENT_MEMORY.md).
The broader design below retains follow-up capabilities and evaluation targets; it is not
a claim that every future consolidation, maintenance, or client acknowledgement feature exists.

## Summary

CodeGraph accepts observations from a calling agent, turns them into evidence-linked memories,
and assembles relevant code and memory into bounded context for future tasks. Reads and
claim-writing operations are always semantic: retrieval operates on meaning, and writing
reconciles the meaning of new claims against existing knowledge. Exact identifiers, lexical
search, and graph traversal support these operations without replacing semantic processing.

The calling agent supplies an observation and, when available, its evidence. CodeGraph owns
claim extraction, classification, anchoring, reconciliation, retention, and context placement.
The calling agent does not need to choose a memory tier, taxonomy, deduplication action, or TTL.
A background LLM proposes classifications and relationships; deterministic policy controls
scope, authority, promotion, and persistence. Uncertainty is preserved when evidence is missing.

Memory-aware operation is an explicit project/client setting. Once enabled, the four public
agentic workflows consult memory automatically through a shared context service. Standalone
memory tools remain available for direct recall, corrections, and forgetting.

## Goals and boundaries

- Preserve useful decisions, preferences, procedures, verified findings, task outcomes, and
  working state across context loss and sessions.
- Ground memories in supplied observations and evidence, with traceable revisions and history.
- Combine current code evidence and historical knowledge within the consuming model's budget.
- Keep the agent-facing interface simple while moving memory management into CodeGraph.
- Recover pending work after interruption and preserve project/user isolation under concurrency.

The first implementation processes deliberately submitted observations. It does not archive
all traffic, capture complete transcripts, or treat CodeGraph's generated answers as independent
evidence. Automatic capture from trustworthy task/session events is a later integration, not
an assumption about access to a client's conversation history. Useful outcome summaries and
small evidence excerpts are allowed; bulk transcript/diff ingestion is outside the initial scope.

## Division of responsibility

| Concern | Owner | Contract |
| --- | --- | --- |
| Supplying an observation | Calling agent or authorized client hook | Report what happened, why, and available evidence; distinguish findings from hypotheses. |
| Capture timing | Calling agent/client integration | Submit at decisions, meaningful outcomes, corrections, and handoffs; CodeGraph cannot infer unseen conversation events. |
| Claim extraction and classification | CodeGraph background worker | Produce atomic claims and proposed kinds, durability, and relationships from supplied context. |
| Code anchoring | CodeGraph | Resolve identifiers and related code; distinguish supporting evidence from topical associations. |
| Scope and authority | Deterministic policy and authenticated client context | A classifier cannot widen access or convert a claim into an instruction. |
| Reconciliation and revisions | CodeGraph | Apply validated decisions transactionally; preserve conflicting claims when unresolved. |
| Retention and tier eligibility | CodeGraph policy | Apply explicit overrides and bounded defaults; classification informs policy without overriding it. |
| Placement in active context | Shared context service | Select applicable memories at read time, under a token budget. |
| Verification and correction | Calling agent or authorized verifier | Supply fresh evidence; an LLM's classification is not verification. |
| Forgetting | Explicit authorized operation | Remove selected content and affected derivatives; preserve unrelated memories. |

## Scope, kind, tier, and state

These are independent dimensions. None is encoded implicitly through wording or free-form tags.

### Scope

- `session`: working knowledge visible within an identified session/task. Session identity comes
  from the client or an explicit argument; CodeGraph does not invent session correlation.
- `project`: codebase decisions, practices, procedures, and outcomes. All records and retrieval
  paths are scoped by a stable `project_id`, including branches and worktrees of that project.
- `user`: explicitly selected cross-project preferences and personal working conventions.
  User content is stored outside repositories and has an owner identity.
- Organization/team scope is a future extension requiring its own sharing and permission model.

An omitted write scope defaults to the current project. Session scope requires a session ID;
user scope must be selected explicitly or authorized by an established client policy. An LLM
may suggest another scope but cannot move content there automatically. Promotion from session
to project requires an authorized policy or explicit operation; tier changes do not widen scope.
Branch, revision, environment, and task applicability are separate from access scope.

### Kind

Start with `fact`, `decision`, `preference`, `procedure`, `episode`, and `working_state`.
`unclassified` is valid while processing is incomplete or uncertain. An observation can produce
several claims with different kinds. Tags remain optional organizational hints.

Facts describe knowledge; episodes describe actions and outcomes; procedures describe reusable
steps and their conditions. Every kind supports semantic search. Semantic memory as a kind of
knowledge must not be confused with semantic search as a retrieval method.

Code anchoring is optional for every kind and scope. Agents may deliberately write memories
about requirements, collaboration, rationale, or other matters without a direct code link.
These memories can become active and remain semantically retrievable without graph anchors;
absence of code grounding alone does not make them provisional, stale, or less credible.

### Logical tiers

| Tier | Typical contents | Serving and lifecycle |
| --- | --- | --- |
| `working` | Objective, constraints, hypotheses, blockers, latest task results | Session/task context and handoff; short review horizon. |
| `durable` | Decisions, useful findings, preferences, procedures, outcome summaries | Recalled across sessions by meaning and applicability. |
| `core` | Small set of established constraints and proven reusable procedures | Eligible for applicable context packs; strict size and authority limits. |
| `archive` | Superseded claims, historical outcomes, inactive working state | Retrieved for history, explanation, and verification. |

Tiers are logical lifecycle/serving policies, not four mandatory physical databases. Kind does
not dictate tier: an episode can be durable or archived; a procedure need not be core.

Write-time classification proposes durability and reuse potential. Read-time policy decides
what enters the current context. Core eligibility requires established evidence or an explicit
authorized constraint, suitable scope, and clear applicability. Frequency of retrieval alone
never establishes truth or promotes a memory. Mandatory applicable constraints supplied by an
authorized policy are retained even if their semantic similarity to the task is low.

The worker assigns an initial tier through policy using the classifier's proposal. Session
working state defaults to working; reconciled project/user knowledge defaults to durable;
superseded or inactive historical claims become archive. Core requires the stronger eligibility
checks above. Failed or uncertain classification remains provisional/unclassified, without
requiring the calling agent to choose a tier or treating a failure as a reason to discard it.

### Independent states

Track processing (`pending`, `running`, `ready`, `failed`), embedding readiness, lifecycle
(`provisional`, `active`, `superseded`, `archived`, `retracted`), evidence state
(`reported`, `verified`, `disputed`), and review/temporal eligibility independently.

A recorded claim is not automatically verified. A ready embedding does not mean classification
completed. An expired applicability window does not delete history. Classification confidence,
anchor confidence, and evidential support are distinct signals, not one interchangeable score.
Track code grounding separately as `anchored`, `unresolved`, `not_applicable`, or `unavailable`.
`unresolved` means an intended code association could not be resolved; `not_applicable` means
no code association is needed; `unavailable` means the required index/source cannot be checked.
None of these states alone changes the claim's evidence state or retrieval eligibility.
Semantic readiness additionally requires compatible index visibility; producing an embedding
alone is insufficient. Operation responses distinguish acceptance, processing completion, and
semantic readiness, including completion with an unresolved conflict.

## Store ownership and persistence

The code index is derived and rebuildable; project memory is durable knowledge. A proposed
layout is `<project>/.codegraph/memory-db` alongside `<project>/.codegraph/db`, with the user
memory store under `~/.codegraph/`. Index rebuild, `--force`, and schema bootstrap must not wipe
memory. Deleting memory requires an explicit memory lifecycle/administrative operation.

Session records live in the appropriate memory store with session isolation. User stores keep
provenance, revisions, relationships, and jobs just like project stores, but have no direct code
record references. All stores support equivalent memory lifecycle semantics.

Memory-to-code links cross the store boundary through stable logical node IDs and source
locators; the service resolves them against the project's current index. They are not foreign
record references requiring a cross-database join, and never cascade on code node deletion.
Historical locators and fingerprints survive when a node disappears or an index is unavailable.

Database-native integration retains this separation: the durable memory database uses
native relations to its own logical code-locator records. The code database holds a
rebuildable, project-scoped, content-free memory projection with local relations to `nodes`. Projection
records are retrieval aids, never the authority for revisions, scope, or forgetting; candidate
IDs must be checked against durable memory before delivery. User/session content is not copied
into the project projection. An unavailable projection does not block semantic recall.
Code rebuilds may remove this projection, while durable locators/history remain intact.
The typed durable outbox records only the latest project-claim reference/anchor change,
including content-free deletion markers. Cursor-paginated synchronization is bounded and
transactional within each store; no atomic transaction across stores is claimed. The code
owner writes local relations, and resets projection cursors when its input fingerprint changes.
Native joins return exact revisions and bounded directed paths; durable retrieval rejects
stale, forgotten, inaccessible, or ineligible references before ranking and delivery.

Use one owning process per embedded store. Project MCP servers access their project owner;
CLI commands attach to an existing owner rather than opening a locked store. Multiple project
servers access the same user store through a shared local user-memory owner. Store ownership,
local IPC discovery, access control, and crash recovery must be implemented before concurrent
user-store use is supported. A per-project lock is not sufficient for a shared user store.
The exact IPC transport is an implementation choice, not permission to open competing writers.

Reconciliation transactions use revision checks and scoped idempotency keys. Concurrent writes
must not silently overwrite claims. Workers re-evaluate changed candidates before committing.
Queues, leases, and operation state are persisted; abandoned leases are recovered after owner
restart. Cross-store operations cannot assume an atomic transaction across both stores.

## Conceptual data model

The logical model is implemented in the typed `agent_memory_v2.surql` schema, with
identity-specific HNSW partitions declared through `agent_memory_vectors.surql`. Native
claim relations, observation derivation, evidence records, and code-locator edges use stable
record identities; v1 payload stores migrate transactionally and commits write changed rows. Both bundled graph schemas must remain consistent wherever graph-side changes
are required. Memory schema migrations have their own version and preserve existing content.

```
memory_observations
  id, original_statement            -- immutable except explicit authorized purge/redaction
  scope, owner_id, project_id, session_id, task_id
  observed_at, recorded_at, written_by, client_id
  applicability                     -- branch/revision/environment/conditions; unknown is allowed
  evidence                          -- source URIs, excerpts, hashes, evidence descriptions
  idempotency_key, processing_state, operation_id
  embedding_identity, embedding_ready

memories                            -- stable identity for an atomic claim
  id, current_revision, statement
  observation_ids, scope, owner_id, project_id, session_id
  kind, tier, lifecycle, evidence_state, tags
  code_grounding                    -- anchored/unresolved/not_applicable/unavailable
  observed_at, recorded_at, valid_from, valid_until
  applicability
  confirmed_at                      -- absent until explicitly re-verified
  review_after, expires_at, purge_after
  classification_confidence, classification_reason
  classification_identity           -- model/revision/prompt/schema/policy identity
  embedding_identity, embedding_ready
  embedding_<dim>                   -- indexed only within a compatible embedding identity

memory_revisions
  memory_id, revision, statement, metadata, evidence
  changed_at, operation_id, reason   -- append-only history until explicit purge

memory_links                        -- claim revision -> code locator
  memory_id, revision, project_id, node_id
  symbol, file_path, start_line, source_revision
  role                              -- 'evidence' | 'related'
  method                            -- exact/contextual/semantic resolution provenance
  confidence, content_fingerprint, semantic_fingerprint
  verification_state, last_checked_at

memory_relationships                -- claim revision -> claim revision
  from_id, from_revision, to_id, to_revision
  kind                              -- equivalent/complements/contradicts/supersedes/derived_from
  evidence, operation_id, created_at

memory_jobs
  operation_id, observation_id, state, stage
  attempt, lease_owner, lease_until, next_attempt_at
  input_revision, processor_identity, last_error
```

Scope, identity, temporal eligibility, and lifecycle filters apply before candidate selection
on every search path. Index full text on natural-language claim text with an analyzer suitable
for prose and code identifiers; evaluate whether `code_text` alone is sufficient. Add indexes
for scope/lifecycle, review deadlines, jobs/leases, code locators, and relationship endpoints.
Reuse dimension dispatch where useful, but dimension alone is never vector-space identity.

Observations, claims, revisions, and evidence require explicit derivation links so corrections,
reclassification, and forgetting can identify affected derivatives. A stored source locator is
not proof that an external source will remain accessible; retain sufficient permitted evidence
or expose that verification is unavailable.

## Embedding contract

Memory uses the existing CodeGraph embedding and reranking pipeline. Reuse
`codegraph-vector::EmbeddingGenerator`, the existing provider abstractions/configuration,
input-policy validation, batching, cache identities, and `Reranker` factory/interface. Adapt
memory observations and claims to those interfaces rather than introducing memory-specific
provider clients, model stacks, or duplicated embedding/reranking configuration. Share runtime
instances where their configuration and embedding identity are compatible; the user store's
fixed identity uses the same factories without inheriting an incompatible project's vectors.

Stored observations and claims use the provider's configured document/passage task, or its
supported symmetric equivalent. Retrieval uses the paired query task. Preserve model-aware
Jina task pairing and explicit supported configuration; do not hardcode a task from another
model generation. Deduplication thresholds cannot assume query/document scores are universally
calibrated equivalence measures.

Identity includes provider/model/revision, actual task and request options, tokenizer/runtime,
dimension, extraction policy, and relevant processing policy. Chunk/prepared-vector caches
must respect the existing input-policy invariants. Validate complete tokenized inputs, preserve
syntax/Unicode where applicable, and never silently truncate oversized observations or claims.

The user store owns a stable embedding configuration independent of whichever project calls
it. When stores use different identities, encode the query for each compatible space and fuse
ranked results with explicit source weighting; do not compare raw cosine scores across spaces.
Model changes require migration/re-embedding with readiness tracked per identity. Semantic
operations report unavailable or partial coverage rather than silently becoming lexical-only.

## Write path: `memory_write`

Minimal agent input:

```json
{
  "statement": "We rejected parallel dot-product reductions because they changed resolution decisions.",
  "evidence": [
    {"uri": "commit:<sha>", "description": "Implementation and regression evidence"}
  ]
}
```

Optional fields include `scope`, session/task identity when not supplied by the connection,
applicability, tags, `replace`, `ttl`, retention/kind hints, and `idempotency_key`. Hints are
validated against policy. `ttl` controls active eligibility, not implicit permanent deletion.
An explicit correction can target an ID or a semantic description of the old claim.

### Foreground acceptance

1. Validate input and authorized scope; capture trustworthy connection/source metadata.
2. Embed the original observation with the storage task and validate provider input limits.
3. Commit the observation, a provisional searchable representation, and a background job
   atomically. Exact replay of the same idempotency key/input returns the existing operation;
   reuse with different input is an error.
4. Return `operation_id`, provisional IDs, `embedding_ready`, and `processing_state`.

The default acknowledgement guarantees durable acceptance and semantic visibility to the
originating session once the compatible embedding/index is ready. This requires waiting for
embedding and index visibility. A client may request asynchronous acceptance; its result must
say `embedding_pending` and cannot claim a completed semantic write. Provider failure returns
an explicit failure or a persisted pending state, never an unreported lexical fallback.

Provisional observations are visible only through an authorized session view or explicit
provisional filter and are clearly labeled. They are not promoted into shared active context
before reconciliation. Once processing completes, replace their retrieval representation with
atomic claims while retaining the original observation as provenance.

### Background processing

1. Extract atomic claims and entities from the observation and supplied context. Preserve
   uncertainty, negation, conditions, and dates; do not invent missing conversation history.
2. Retrieve bounded candidates using semantic, lexical, and code-anchor signals. Reconciliation
   candidates must have the same owner and access scope as the new claim. Other authorized
   scopes can provide contextual evidence but cannot be mutated or used to suppress a claim
   in its selected scope. Include applicable history when needed to interpret a correction.
3. Use a bounded LLM call to propose kind, durability, reuse potential, review conditions,
   relationships, and core eligibility. Re-query candidates for extracted claims when the
   original observation's search was insufficient.
4. Resolve code anchors, with exact/contextual resolution before semantic association. Store
   unresolved identifiers as text; preserve ambiguity rather than selecting a convenient node.
   Record grounding status; a missing or inapplicable anchor does not block activation after
   semantic reconciliation. Do not manufacture code links for memories unrelated to code.
5. Validate structured output, candidate IDs, evidence references, and scope. Deterministic
   policy decides permissible changes; re-embed derived claims and check candidate revisions.
6. Commit claims, revisions, relationships, links, and job completion transactionally. Publish
   active semantic readiness only after their compatible indexes are visible.

The classifier receives observations, evidence, metadata, and retrieved candidates. It has no
implicit access to full conversations. Model output is a proposal, not an arbitrary database
query or proof that a claim is correct. Bounded retries, backoff, deadlines, concurrency, and
cost limits apply independently from embedding limits. Failed jobs remain recoverable and
observable; embeddings-only deployments retain provisional observations but cannot claim full
automatic classification or reconciliation.

### Semantic reconciliation rules

| Relationship | Action |
| --- | --- |
| Equivalent | Retain the claim and attach provenance; repeated assertions do not count as independent verification. |
| Complementary | Keep both claims and connect them; preserve their conditions. |
| Contradictory | Keep a conflict unless evidence/policy resolves it; expose disputed status. |
| Explicit correction | Supersede the selected claim revision with authorized replacement evidence. |
| Unrelated | Create a new claim. |

Similarity generates candidates; it never alone authorizes deduplication or supersession.
"Truncation is enabled" and "truncation must remain disabled" must survive candidate generation
as distinct claims. Compare entities, kind, branch/version, temporal overlap, and source
support before interpreting a contradiction. Newness alone is not authority. Explicit
corrections with ambiguous semantic targets return candidates for disambiguation; they do not
supersede every nearby memory.

## Read path: `memory_read`

```json
{
  "query": "What constraints and previous decisions matter when changing semantic scoring?",
  "scope": "both",
  "token_budget": 3000,
  "limit": 10,
  "include_stale": false,
  "include_needs_verification": true,
  "include_expired": false,
  "include_archived": false
}
```

`both` means the authorized current project and user scopes; session context is added only
when the client supplies an authorized session identity. Optional inputs include applicability,
`as_of`, `since`, `symbol`, `node_id`, kind filters, and `include_provisional`. A query remains
required; symbols/IDs constrain candidates without replacing semantic retrieval.
`since` filters recording time; `as_of` selects the asserted validity interval. Historical mode
admits superseded/archived claims applicable at that time and labels later corrections; ordinary
reads use current lifecycle eligibility. A later extension can add a separate `known_at` cutoff
for questions about what the store knew at a past recording time.

After scope and eligibility filtering, retrieve three ranked lists:

1. Semantic neighbors over compatible memory embeddings.
2. BM25 over claim text, including precise identifiers and version strings.
3. Memories linked to semantically relevant code nodes and bounded, typed graph neighbors.

Unanchored memories participate through semantic and lexical retrieval. An empty graph list
is normal for them, not a partial-result failure. Rank their relevance and evidence on their
own merits; code-link count must not become a general credibility or inclusion requirement.

Fuse with RRF, then assess task relevance, applicability, evidence state, conflicts, and
redundancy. Bound graph expansion and candidate lists. Cross-store fusion identifies which
lists originate from each store so extra lists cannot accidentally dominate ranking. Do not
interpret RRF or cosine scores as truth probabilities.

A read need not invoke an LLM: semantic retrieval, deterministic policy, and the existing
configured reranking stage serve the normal path. Rerank fused memory candidates through the
shared `Reranker` interface using claim text and bounded relevant context, preserving candidate
IDs and provenance. Honor the existing reranking provider, enablement, and request/input limits;
do not introduce a second reranking stack. If reranking is disabled, retain fused ranking;
if a configured stage fails, expose that failure or a declared partial-result policy.
Return warnings about unavailable sources, unready embeddings, missing applicability, or
provisional evidence. Read frequency updates
usage telemetry only; it does not confirm truth, refresh evidence, extend TTL, or promote tier.

Results include claim/revision IDs, statement, scope/kind/tier, evidence state, provenance,
temporal/applicability fields, review status, conflicts, and anchors. Show current code locations
when resolved and historical locators otherwise. Historical reads can select superseded claims
valid at the requested time rather than always returning their successor.

### Needs verification

Return a separate `needs_verification` collection for highly relevant memories whose supporting
evidence changed, disappeared, or requires renewed verification. Unresolved conflicts also
belong here when they matter to the task. These entries are warnings or historical context,
not established current facts. This collection is enabled by default in memory-aware agentic
workflows, including context and architecture as well as impact and quality. Standalone reads
share the same contract through `include_needs_verification`.

`include_stale: false` excludes stale claims from the ordinary `memories` collection; it must
not hide relevant warnings from `needs_verification`. Run a bounded semantic candidate search
over review-required claims using the same scope/applicability checks and shared ranking
pipeline. Authorization, explicit retraction/forgetting, and temporal/lifecycle boundaries
still apply; this warning path does not automatically include all expired or superseded claims.
Expired/superseded claims require the requested historical view or explicit inclusion policy.

Each warning carries the claim/revision, original evidence, review reason, known evidence
changes, and a suggested verification target. Include historical and current code context
when available, with explicit snapshot labels. A missing anchor for a deliberately non-code
memory is not a verification warning by itself. Evidence state remains reported/verified/
disputed independently; a review warning does not prove that the claim is false.

Both collections are relevance-selected and budgeted. Avoid duplicating a review-required
claim as an apparently current fact in ordinary results. Return counts/truncation metadata
when relevant warnings cannot fit, and preserve important warning summaries before optional
surrounding graph detail. Reading or citing a warning does not clear its review requirement.

### Code snippets and graph paths in results

Every memory result, including one injected into an agentic tool result, carries a grounding
status and retrieval explanation. Code-related results additionally carry a bounded code
context when the relevant source is available. Standalone reads and automatic retrieval share
this contract; the calling agent does not need to reconstruct relationships from bare IDs.

Distinguish two kinds of path:

- Evidence paths connect the claim to its supporting code/test/document anchors. A related
  anchor is labeled as topical context, not as proof of the claim.
- Retrieval paths explain how the query found the memory: a matched code node, traversed code
  edges, and the final memory anchor. Direct semantic or lexical matches need no graph path;
  record their retrieval method without inventing an edge for vector similarity.

Code context includes stable node/edge IDs, symbols, file spans, edge types and directions,
resolution provenance, source revisions/fingerprints, and short supporting snippets. Define
`code_hop_count` as the number of traversed code edges; the memory anchor is reported separately.
Preserve whether an edge is exact, contextual, or inferred. Graph proximity alone does not
verify a statement, and an inferred path must not appear as an established call relationship.

Default to direct anchors, short evidence spans, and selected paths with at most one code hop.
Deeper expansion is optional and task-sensitive, especially for impact analysis. Bound paths,
nodes, edges, source bytes, and total context tokens independently. Return truncation metadata
and locators for omitted detail; deduplicate repeated nodes/snippets in a shared context pack
and let memory results reference them. Preserve the memory and its grounding status even when
the code-context budget is exhausted.

Keep the original evidence locator/fingerprint at write time and resolve current graph context
at read time. Source snippets identify the snapshot/revision they came from. A current source
read and a stale indexed location must not be combined as if they describe the same snapshot;
detect fingerprint mismatch and report changed or unavailable evidence. Historical evidence
can accompany current context but is labeled separately and is not silently replaced.

For non-code memories, return `not_applicable` with empty code-context arrays; for an unresolved
intended anchor, return `unresolved` with the supplied identifiers and provenance. Both remain
retrievable and eligible for task-relevant context. Missing code does not erase evidence from
other sources or imply that the original observation was false.

## Context assembly and agentic integration

The shared context service combines semantic memory retrieval with current code evidence for
`agent context`, `agent impact`, `agent architecture`, and `agent quality`. Initial rollout
integrates one workflow, then extends the same service to all four. Registration and shared
budget changes must preserve parity between ReAct and LATS and branch-local transcripts.

### Retrieval timing and workflow focus

Automatic retrieval informs reasoning and is exposed in the final tool result:

1. Before reasoning, retrieve memories and verification warnings using the task query and
   authorized scope/session/applicability. Supply the resulting context to the internal agent.
2. During code discovery, use newly found symbols/nodes and relevant graph paths to retrieve
   additional memories through the same semantic service. Supply meaningful additions before
   finalizing the answer; appending them only after answer generation is insufficient.
3. Return the selected memories and warnings alongside the answer and code findings, retaining
   IDs, revisions, provenance, snippets, and graph paths under the shared result budget.

| Workflow | Ranking emphasis |
| --- | --- |
| `context` | Local conventions, procedures, task history, and implementation decisions. |
| `impact` | Behavioral guarantees, compatibility constraints, and previous failed changes. |
| `architecture` | Design rationale, rejected alternatives, and architectural constraints. |
| `quality` | Investigated findings, deliberate tradeoffs, and unresolved risks. |

These are ranking preferences, not exclusive kind filters. Non-code memories can be relevant
in every workflow. Selection may return no memories; never fill a quota with weak matches.

Bound discovery-triggered searches, cumulative candidates, graph expansion, reranking calls,
and total retrieval time in addition to output tokens. Not every discovered node triggers a
new search. Reuse compatible embeddings, retrieval results, and reranker instances with cache
keys covering scope, applicability, memory revisions, code-index generation, and processing
policy. Invalidation must prevent a corrected/forgotten claim from returning through a cache.
All foreground retrieval stays within the agent's existing whole-workflow deadline.

### Returned memory contract

Public CLI/MCP responses preserve the existing answer/findings contract and add a structured
memory context with `memories`, `needs_verification`, `retrieved_memory_refs`,
`cited_memory_refs`, and retrieval metadata. References include claim ID and revision. Cited
references identify memories explicitly cited by the selected answer, not inferred attention
or assumed influence. Returned references must resolve to included items or an explicit
locator/omission record, with withdrawn references removed after mid-workflow invalidation.

Retrieval metadata distinguishes `ok`, `empty`, `disabled`, `unavailable`, and `partial`, and
records query/discovery triggers, selected counts, budgets, truncation, and source failures.
An empty collection after successful retrieval is different from an inaccessible memory
store. Final summaries and structured collections share the same eligibility and evidence
labels; the answer must not present a needs-verification claim as freshly verified knowledge.

Deduplicate claim revisions and shared code context within a workflow. Across calls, suppress
repeated delivery only with explicit client session/context-epoch and delivery acknowledgement;
the server cannot assume earlier results remain in the client's context after compaction.
Changed revisions and verification warnings must remain eligible for renewed delivery.

ReAct and LATS share the retrieval capability and consistent starting memory snapshots.
Candidate-specific discoveries and observations stay in branch-local transcripts; unverified
candidate conclusions are not promoted to shared evidence or persisted automatically. Record
retrieval/citation traces per candidate and attribute final citations to the selected answer.
Readiness and corrections arriving during a workflow are handled through revision checks and
reported snapshot changes rather than silently mixing incompatible claim versions.

A context pack contains applicable constraints, relevant decisions/procedures, useful episodes,
authorized working state, current code findings, and conflicts needing verification. It keeps
sources distinct: remembered decisions, reported outcomes, and currently verified code facts
are labeled rather than flattened into one apparently authoritative narrative.
When memory and current code disagree, expose the discrepancy in `needs_verification`; do not
silently choose either source or treat historical rationale as proof of current behavior.

Budget the complete pack with the consuming model's tokenizer, including wrappers, citations,
and reserved workflow context. The embedding tokenizer is not the context-budget tokenizer.
For unknown model tokenizers, use a declared conservative estimate with headroom and expose
that limitation. Remove redundant items, preserve qualifications/negation, and retain links to
supporting detail. Applicable mandatory constraints have a reserved budget; if they cannot fit,
return an explicit budget error rather than silently dropping them.

Memory is retrieved data with its original authority. Learned procedures do not override user
instructions, repository rules, or client permissions. Core eligibility is not permission to
execute steps. Memory-aware integration is visible in workflow metadata and can be disabled;
once enabled, the caller does not need a separate memory lookup before every agentic question.
Memory-stage failures are reported as partial context without implying that memory was absent.
An answer produced using a memory is not independent evidence confirming that memory. Repeated
retrieval, citation, or model paraphrasing never increases its evidential confidence.

## Update, confirmation, and forgetting

### `memory_update`

Accept an `id` or semantic `target` plus a correction, metadata changes, or explicit `confirm`.
IDs are precise selectors; changed statements still pass through semantic reconciliation.
Natural-language targets use the read pipeline and require disambiguation when several claims
fit. Updates include an expected revision to prevent overwriting concurrent changes.

Statement corrections create a new revision with fresh embeddings and anchors. A different
claim uses a new identity and a supersession relationship. Metadata changes do not confirm
truth, clear stale evidence, or extend retention. No-op calls with only an ID are not implicit
confirmations. `confirm` requires a declared re-verification and evidence references; update
only the evidence actually checked, then recompute review status and policy deadlines.

Accept optional explicit usefulness feedback through `memory_update` without changing claim
truth, confirmation time, review requirements, or TTL. Useful/not-useful signals can inform
ranking; an outdated/incorrect report flags review or accompanies a correction with evidence.
Track feedback provenance and deduplicate repeated feedback from the same operation so repeated
delivery is not counted as independent validation.

Explicit TTL changes take precedence over inferred policy. The resulting expiry is returned;
there is no second hidden extension by a scope default. Confirmation extends a review window
according to the stored policy without replacing a custom TTL or clearing unrelated conflicts.

### `memory_delete`

Accept an `id` or semantic `target`. Resolve and return ambiguity before deleting; a broad
query is not authorization to delete all neighbors. Selected IDs can be submitted together
for an explicitly intended multi-item operation.

Remove selected claims, their revisions/embeddings/links, and affected derived content. Rebuild
summaries or profiles from surviving sources; deleting a parent must not leave its statement
recoverable through a derivative. For an observation supporting several claims, purge or redact
the relevant source content and re-derive surviving claims as necessary. Immutability yields
to explicit authorized forgetting; audit metadata must not retain the forgotten text.

Cancel pending jobs and use generation/tombstone checks so an in-flight worker cannot restore
deleted content. Retain only content-free operation identifiers needed for replay protection.
Do not resurrect superseded predecessors. Cross-store purge reports each store's completion
and resumes partial work rather than claiming a transaction spanning both stores.

## Temporal validity, review, and retention

Keep separate concepts:

- `observed_at`: when an event or observation occurred; absence remains explicit.
- `recorded_at`: when CodeGraph accepted it.
- `valid_from` / `valid_until`: the asserted interval of applicability; never inferred from
  recording time alone.
- `review_after`: when fresh verification is due; overdue does not automatically mean false.
- `expires_at`: when ordinary active retrieval stops, based on explicit temporary applicability.
- `purge_after`: when retention policy permits permanent deletion.

Starting policies are configurable hypotheses to evaluate: working state gets a short,
task-sensitive review horizon; temporary workarounds can expire; durable facts are reviewed
according to volatility; decisions and useful outcomes become historical rather than being
hard-deleted after 90 days. User preferences remain until corrected or explicitly forgotten,
with reviews where their applicability warrants it. Core summaries are refreshed when their
sources change. Archive retention is a separate quota/retention policy, not an automatic
consequence of expiry or deleting the newest successor.

Eligibility is checked at read time even if maintenance has not run. Maintenance at store open,
on a bounded schedule, and after re-index updates review state and performs authorized retention
work. Reading alone never extends life. Automatic permanent deletion is limited to explicit
purge policies; unresolved conflicts and durable decisions are not silently discarded by a
universal scope TTL.

## Code evidence and staleness

Re-index compares anchor fingerprints and source applicability, not only `nodes.updated_at`.
Evidence anchors contribute to review state; related anchors contribute to discovery. A change
to merely related code must not invalidate a claim. An evidence change means re-verification
is needed, not that the claim was proved false.

Record content hashes and, where defined and versioned, semantic fingerprints of the supporting
artifact. Fingerprints must include relevant build/config/documentation inputs. If dependency
coverage is incomplete, report that limitation rather than implying unchanged code proves the
claim valid. Keep branch/revision context and historical locators through rename or deletion;
re-anchoring must preserve provenance and avoid silently attaching a different symbol.

Represent reasons such as changed evidence, missing evidence, out-of-date verification, or
unknown applicability. A node disappearing from the index and a source being deleted are
separate possibilities. User memories have no code anchors but can still need review, conflict
resolution, or temporal invalidation through new explicit observations.

## Consolidation and core promotion

Write-time extraction and reconciliation are required. Broader background consolidation is a
later stage, after correctness and task utility are measured. It may derive scoped profiles,
procedure summaries, or summaries of related episodes, with links to all source claim revisions.

Derived summaries are replaceable views. Preserve source claims, qualifications, evidence, and
conflicts; never train future confidence on repeated model-generated summaries of the same
source. Refresh or invalidate a view when its sources change. Promotion into core requires
policy-backed evidence and applicability, not simply an LLM label or high retrieval count.

## MCP and CLI surface

The initial surface remains four tools in the official server:

- `memory_write`: semantic acceptance, classification, and reconciliation.
- `memory_read`: semantic recall and optional budgeted context pack.
- `memory_update`: semantic correction or explicit evidence-backed confirmation.
- `memory_delete`: precise or semantically selected forgetting.

Expose equivalent `codegraph memory write|read|update|delete` commands. Add operational status
and retry/wait support for background jobs without confusing job inspection with memory recall.
Claim writes need embeddings; automatic extraction/reconciliation also needs a configured LLM
feature/provider. Embeddings-only mode exposes its processing limitation explicitly.

CLI and MCP share policy, selection semantics, scope enforcement, and store ownership. Preserve
existing whole-workflow deadlines and shared result budgets when memory is integrated into
agentic commands; background work has its own bounded deadline rather than extending a running
agent's budget indefinitely. Errors and partial readiness are machine-readable.

## Guidance for calling agents and hooks

Submit observations when a decision was made, a useful outcome or failure occurred, a finding
contradicted prior understanding, a preference was explicitly expressed, or a task needs a
handoff. Include what happened, conditions, and source evidence when available. Do not present
an untested plan, generated answer, or hypothesis as a verified result.

Agents may submit temporary blockers/objectives as session working state; they need not promote
progress notes into durable project knowledge. They do not need to assign tiers, classification,
or TTLs. Explicit expiry and scope overrides are useful when the agent knows the intended
boundary. Client hooks can supply task/session/revision metadata and submit authorized compact
outcomes, without assuming automatic access to unseen conversation history.

When memory-aware workflows are enabled, they supply relevant context automatically. Use direct
reads for additional questions or historical investigation. Verify disputed or changed evidence
before relying on it; submit a correction or explicit confirmation with the new evidence.

### Memory guidance installed by `codegraph init`

Extend the existing project instruction generator in
[`project_init.rs`](../../crates/codegraph-mcp-server/src/project_init.rs) so `codegraph init`
adds or refreshes memory-usage guidance in both project `AGENTS.md` and `CLAUDE.md`. Reuse the
existing `<!-- codegraph:begin -->` / `<!-- codegraph:end -->` managed block and merging logic;
preserve unrelated instructions and hooks, keep repeated initialization idempotent, and leave
user-level agent settings untouched. Instruction updates apply even with `--hooks none`.
`--no-index` must still install guidance without loading providers, initializing models, opening
memory stores, or performing memory operations. See [init usage](../AGENTIC_CLI.md).

The generated guidance explains when to write observations, how to read/correct/confirm/forget
memories through MCP tools or equivalent CLI commands, and how memory-aware agentic workflows
already supply context. Include concise examples and the current project identity. Explain
that CodeGraph handles tiers/classification, non-code memories are valid, code-related results
carry snippets/graph paths when available, and evidence must accompany verification claims.
Describe provisional/pending writes and unavailable capabilities without promising completed
processing, and teach agents to inspect `needs_verification` before relying on historical
warnings or changed evidence. Teach explicit user/session scope selection and direct semantic
recall when extra context is needed; do not require the calling agent to manually duplicate
automatic retrieval.

Guidance must match the capabilities shipped at each implementation stage. Both harnesses use
the same policy and tool contracts. Existing project-local hooks can remind agents at appropriate
task boundaries, but instruction installation itself does not enable transcript capture or
perform writes. Keep the repository guidance templates and `codegraph agent instructions`
consistent with the generated managed block.

## Implementation sequence

1. Define operation/readiness contracts, scope policy, source/claim revisions, semantic target
   selection, and isolated fixtures. Establish separate durable memory storage, migrations,
   project ownership, and a concurrent user-store ownership protocol.
2. Deliver a vertical slice: semantic acceptance, persistent jobs, bounded background claim
   extraction/reconciliation, source-linked claims, and semantic recall. Include interruption,
   provider failure, ambiguity, and revision-conflict handling from the start.
3. Integrate a budgeted context pack into `agent context`; then share the same service with
   impact, architecture, and quality. Include pre-reasoning/discovery-triggered retrieval,
   structured memory results, and `needs_verification`, retaining budgets and ReAct/LATS parity.
4. Add evidence fingerprints, temporal reads, review/retention maintenance, explicit confirmation,
   correction, and complete forgetting through derived views and pending jobs.
5. Extend `codegraph init` guidance for both instruction files, client guidance and project-local
   hooks; add working-state handoff and measured core eligibility.
6. Evaluate task utility and cost before introducing broader capture or background consolidation.

Each delivered stage must expose unavailable/pending capabilities rather than advertising the
full proposed design. No stage requires indexing the working repository or live provider tests;
use isolated stores and mock providers for implementation contracts.

## Validation and evaluation

Offline correctness fixtures must cover:

- Paraphrased recall; procedures and failed approaches; preservation of qualifiers and negation.
- Opposite claims remaining distinct; equivalent claims adding provenance without invented proof.
- Branch/version/environment separation, historical validity, and contradictory source evidence.
- Related-code changes without false invalidation; supporting changes and missing anchors.
- Active unanchored project/user memories recalled semantically and included in applicable
  tool results without fabricated code links or an automatic credibility penalty.
- Evidence versus retrieval paths, directed hop counts, inferred-edge labels, snapshot-matched
  snippets, graph-context budget truncation, and deduplicated context-pack references.
- Model/task/embedding-identity changes, incompatible dimensions/spaces, tokenizer limits, and
  explicitly partial retrieval when indexes are not ready.
- Memory embedding and reranking through the existing shared pipeline with mock providers;
  configured reranking enablement, stable candidate identity, provider limits, and failures.
- Read-after-acceptance guarantees, asynchronous pending responses, worker crash/recovery,
  concurrent correction, idempotent replay, and cancellation after forgetting.
- Scope isolation across sessions/projects/users, competing project servers sharing user memory,
  and CLI attachment to an existing owner.
- Metadata edits without confirmation, explicit TTL precedence, read-time expiry filtering,
  and purge propagation through revisions, summaries, source content, and queued jobs.
- Context budgets with complete wrappers/citations, source attribution, required constraints,
  unresolved conflicts, and parity across public workflows and agent search strategies.
- Relevant changed-evidence memories in `needs_verification` despite `include_stale: false`,
  lifecycle/scope isolation on the warning path, and non-code memories without false warnings.
- Pre-reasoning and bounded discovery-triggered retrieval, empty versus unavailable results,
  cited versus retrieved references, and warning/code discrepancies preserved in the answer.
- Context-epoch delivery deduplication, corrected/forgotten-memory cache invalidation,
  branch-local retrieval traces, and explicit feedback without implied verification.
- `codegraph init` memory guidance in both `AGENTS.md` and `CLAUDE.md`, idempotent managed-block
  updates, preservation of unrelated instructions/hooks, and provider-independent `--no-index`
  setup including `--hooks none`; user-level settings remain untouched.

Evaluate semantic recall/precision, incorrect merge/supersession rates, conflict handling,
staleness false positives, foreground latency, time to active semantic readiness, context size,
background backlog, and inference cost separately. Include coding-task comparisons against
code-only retrieval, retrieval without classification, and the complete memory-aware workflow.
Offline fixture success establishes contracts; it does not establish live task accuracy or
production speedups. Broader consolidation must demonstrate additional utility under a fixed
context/cost budget before adoption.

## Open implementation choices

The initial implementation resolves owner transport to private authenticated loopback IPC,
project identity to a persisted UUID shared by Git worktrees, and background processing to
the existing resolved Rig provider/model. Extraction and reconciliation share one typed-JSON
or proposal-validation repair allowance per attempt, with a 120-second attempt deadline,
up to three automatic attempts for transient failures, and bounded retry backoff.
Missing/incompatible configuration remains explicitly recoverable.
Validation checks claim cardinality, evidence indexes, candidate revisions, and relationship
authorization before accepting either stage. Explicit corrections pass a service-authorized
target and require one replacement claim; only the service mutates that target. Persisted
validation diagnostics are specific and content-free; provider errors remain redacted.
Failed operations retain provisional claims that can be explicitly recalled and forgotten.
An operation-specific read scopes semantic candidates to that authorized operation's
current claims before retrieval. Review entries compact code context against reserved
capacity, preserving current-source excerpts and the full claim's conditions. Extraction
uses the submitted assertions; supporting code evidence does not add unsolicited claims.
Anchor discovery combines semantic candidates with bounded exact-symbol candidates,
scoped by project and supplied file evidence. Only unambiguous, current local code
locations attach; missing code grounding is reviewable and does not invalidate
non-code memory.
The retrieval streams use scoped HNSW/BM25 and native locator-edge candidates,
database-side normalized `search::rrf()` through `fn::memory_fuse`, existing reranking,
and bounded one-hop code context. Vector chunks collapse to claim identities before fusion;
ties resolve deterministically before truncation. Historical versions retain their own
candidate computation while using the same database-side fusion. Review horizons and core rules are documented
in the usage guide. Remaining choices include measured threshold/weight calibration,
retention quotas, proactive index-event maintenance, semantic anchor remapping, and large-store cache sizing and incremental persistence performance.

- Background model selection, structured-output mechanism, batching, retries, and cost budgets.
- Candidate counts, fusion weights, reranking, and confidence calibration by relationship type.
- Review horizons, retention quotas, and explicit evidence requirements for core eligibility.
- Local owner discovery/IPC transport and lifecycle for the shared user-memory service.
- Stable project identity across moves/clones and branch applicability for uncommitted work.
- Which semantic fingerprints capture supporting behavior without overlooking config/build inputs.
- How clients expose operation progress, disambiguation, and verification/forgetting results.
- Session handoff boundaries and authorized automatic capture from client task events.

## Design references

These sources inform the proposal; their benchmarks do not establish CodeGraph performance.

- [MemGPT: Towards LLMs as Operating Systems](https://arxiv.org/abs/2310.08560): separating
  persistent memory from the limited active context window.
- [LangGraph memory overview](https://docs.langchain.com/oss/python/concepts/memory): memory
  kinds, semantic retrieval, and foreground/background processing tradeoffs.
- [Zep: A Temporal Knowledge Graph Architecture for Agent Memory](https://arxiv.org/html/2501.13956v1):
  provenance-linked facts and separate temporal validity and recording history.
