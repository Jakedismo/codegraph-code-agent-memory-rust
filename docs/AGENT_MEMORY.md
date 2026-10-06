# Semantic agent memory

CodeGraph memory stores agent observations independently of the rebuildable code index.
It uses the configured embedding provider, reranker, and LLM. Build the CLI with
`--features full` for classification and agent integration; `--features memory` alone
provides the memory transport/storage with the providers enabled in that build.

## Configure and enable

Use your existing CodeGraph embedding, reranking, and LLM configuration. Memory has
no separate provider credentials. Automatic retrieval is opt-in:

```toml
[memory]
enabled = true
token_budget = 3000
limit = 10
```

`CODEGRAPH_MEMORY_ENABLED=true` overrides the project setting. Each public agent
command accepts `--memory auto|on|off`, `--session-id`, and `--context-epoch`:

```sh
codegraph agent context "Explain the scoring constraints" --memory on
codegraph agent impact "What would parallel scoring affect?" --memory on
codegraph agent architecture "Explain indexing decisions" --memory on
codegraph agent quality "Assess known indexing risks" --memory on
```

MCP exposes `memory_write`, `memory_read`, `memory_update`, and `memory_delete`.
The four consolidated agent tools accept `memory_enabled`, `session_id`, and
`context_epoch`. ReAct and LATS keep discovery results in the branch that received
them. These memory tools do not expand the internal eight-tool graph registry.

## Write and recall

Run from the project root or supply `--project /path/to/project`. Simple observations
and queries can be positional. Structured requests use `--input file.json` or
`--input -` for stdin; paths, including a global `--config` path, are resolved against
the invoking directory before selecting the project.

```sh
codegraph memory write "Preserve scalar summation order in semantic scoring"
codegraph memory read "What scoring guarantees should I preserve?"
codegraph memory write "Prefer short explanations" --scope user
codegraph memory write "Investigating the scoring regression" \
  --scope session --session-id task-42
codegraph memory write "Preserve scalar scoring order" --idempotency-key scoring-42
codegraph memory read "What scoring constraints apply?" --limit 5 --token-budget 2000
```

Use `codegraph memory --help` and each subcommand's `--help` for the CLI surface.
Write flags include `--ttl-seconds`, `--asynchronous`, and `--code-related`. Read
flags include `--scope both|project|session|user`, `--symbol`, `--node-id`,
`--include-provisional`, `--include-archived`, `--include-expired`, and
`--include-stale`. Verification warnings remain enabled. Explicit CLI fields
override their counterparts in `--input`; omitted flags preserve JSON fields.
Evidence, applicability, confirmation, and detailed temporal filters use JSON input.

The default write scope is `project`. `user` is an explicit cross-project scope;
`session` requires an explicit session identity. Reads default to authorized project
and user memory, with authorized session memory when a session is supplied. JSON
reads can select `scope: "both"|"project"|"session"|"user"`, `kinds`, `symbol`,
`node_id`, temporal filters, limits, and a token budget. Applicability conditions
are matched against the supplied client context rather than silently widened.

An evidence-linked request looks like:

```json
{
  "statement": "Scalar scoring order preserves deterministic resolution",
  "idempotency_key": "scoring-investigation-42",
  "evidence": [{
    "uri": "file:crates/codegraph-mcp/src/semantic_scoring.rs",
    "description": "Observed in the implementation and regression test",
    "fingerprint": "sha256:<actual full-file SHA256>"
  }],
  "code_related": true
}
```

Ordinary writes first persist an observation, a provisional semantic embedding, and
a durable processing operation. The worker then extracts atomic claims, retrieves
semantic candidates, reconciles relationships, embeds the resulting claims, and
commits them. A write response is acceptance, not completed classification or proof
of truth. `asynchronous: true` also defers embedding; such observations cannot be
returned through a lexical-only shortcut.

Statements are limited to 64 KiB; the complete observation/evidence/anchor wrapper
is limited to 96 KiB. Reconciliation ranks at most 100 candidates and includes only
whole candidates fitting its input allowance. It never truncates a claim to make
its qualifiers disappear.

```sh
codegraph memory status <operation-id>
codegraph memory wait <operation-id> --timeout-secs 120
codegraph memory retry <operation-id>
```

Use `--scope user` for user-store operations. Job states are `pending`, `running`,
`complete`, `failed`, `config_required`, and `cancelled`; embedding readiness is
reported separately. Without a configured LLM, accepted semantic observations stay
provisional. They are visible to the explicit originating session, or through
`include_provisional: true`, while classification awaits configuration.

The classifier proposes kinds and relationships; deterministic policy chooses
`working`, `durable`, `core`, or `archive`. Session/working-state memory receives a
24-hour review horizon. Facts/procedures receive 30 days, decisions/episodes 90 days,
and preferences no automatic review deadline. These are review horizons, not purge
deadlines. Explicit `ttl_seconds` takes precedence; completion archives working
state. Repetition and usefulness feedback do not establish verification or promote
a claim. Core requires an explicitly authorized, evidence-linked constraint, or an
explicitly confirmed reusable procedure without unresolved conflicts.

## Grounding and verification

Free-text, non-code memories remain semantically retrievable with
`grounding: "not_applicable"`. Code-related memories can have supporting anchors
or discovery associations; semantic proximity creates an association, not proof.
Reads enrich available anchors with bounded project-scoped nodes, directed edges,
resolution provenance, one-hop paths, and snippets. Historical evidence and current
index/source snapshots have distinct labels. Full supporting spans are hashed even
when their displayed snippets are shortened.

An unavailable/locked code index does not prevent durable memory processing.
Grounding can be retried on a subsequent read. Fingerprint mismatches prevent stale
indexed locations from being presented as current source evidence. Relevant changed,
disputed, or review-due claims appear in `needs_verification`, including when
`include_stale` is false. Confirming evidence requires an explicit update; reads and
feedback never clear review requirements.

Semantic and BM25 candidates are filtered by authority/scope/applicability before
selection, fused with graph association signals, and passed to the existing reranker
when configured. A reranker failure returns declared partial semantic results.
An embedding failure or incompatible identity cannot become lexical-only recall.

## Correct, confirm, complete, or forget

An update/delete with a semantic `query` returns `needs_selection`. Select the exact
`memory_id` and `expected_revision` before mutating it:

```sh
codegraph memory update --query "scoring constraints"   # select candidates
codegraph memory update "Corrected observation" --memory-id <id> --expected-revision 1
codegraph memory delete --query "obsolete scoring decision"   # select candidates
codegraph memory delete --memory-id <id> --expected-revision 2
codegraph memory update --memory-id <id> --expected-revision 3 --complete
```

The exact selector requires both identity and revision. A semantic selector never
changes or deletes a claim. `--complete` applies only to session/working-state memory.
The same selectors are available in JSON:

```json
{"memory_id":"<id>","expected_revision":1,"statement":"Corrected observation"}
```

Corrections retain the claim identity, preserve revision history, and create a new
semantic processing operation. Confirm separately with `confirm: true` and evidence
containing explicit `verified_at` timestamps. `reusable: true` applies only to a
confirmed conflict-free procedure. `complete: true` archives working/session state.
Feedback uses `useful: true|false` and an idempotent `feedback_id`.

```sh
codegraph memory update --input correction.json
codegraph memory delete --input selected-memory.json
```

Forgetting purges the selected claim, its revisions, links, vectors, and raw source
observations that contain it. Independent sibling claims remain. Content-free
tombstones/idempotency digests prevent replay; cancelled in-flight jobs cannot
restore forgotten content. Backups made outside CodeGraph are separate copies.

## Context and owner lifecycle

Automatic retrieval runs before reasoning and on bounded discoveries in tool
results: at most two discovery rounds per branch and eight per workflow, sharing a
20-second foreground allowance. The existing whole-workflow deadline still applies.
Memories are never a substitute for successful graph-tool observations in LATS.

Responses add `memory_context` with `memories`, `needs_verification`, status,
truncation/budget metadata, `retrieved_memory_refs`, and `cited_memory_refs`. Exact
citations use `[memory:ID@REVISION]`. Final responses validate delivered revisions;
changed/forgotten references are withdrawn with a warning. Full serialized wrappers
and possible citations count against the consuming-model budget. Use
`CODEGRAPH_CONTEXT_TOKENIZER_PATH` for a local tokenizer when model recognition is
unavailable; the fallback conservatively counts one UTF-8 byte per token. Applicable
mandatory constraints that cannot fit cause an explicit budget error.

A single on-demand OS-user process owns the stores, discovered through private
authenticated loopback IPC. Project memory lives in `.codegraph/memory-db`; a
persisted UUID preserves identity across moves, and Git worktrees share the common
repository's memory. User memory lives in `~/.codegraph/user-memory-db`. User
embedding settings are pinned independently of calling-project configuration.
Memory schema v1 and vector partitions are independent of graph schema/re-indexing.

```sh
codegraph memory service status
codegraph memory service stop
codegraph memory reembed                 # explicit project migration
codegraph memory reembed --scope user    # explicit user-store migration
```

Accepted jobs continue after the client exits. The owner idles out after five minutes
without executable work. Its durable registry contains paths, not credentials; after
a restart, a compatible client must register provider configuration to resume jobs.
Incompatible classifier jobs report `config_required` without starving compatible
work. Re-embedding commits all current and historical vectors atomically; failures
leave the previous generation intact. `CODEGRAPH_MEMORY_HOME` isolates user/runtime
state for fixtures or explicitly separate installations.

The existing provider pipeline has process-level tokenizer/prefix/request controls.
Clients whose controls differ from the owner are rejected explicitly; use `memory
service stop` and reconnect under the intended settings rather than mixing tokenizer
or task identities. Ordinary project provider/model settings are registered separately.

`codegraph init --hooks none --no-index` adds memory usage guidance to both managed
instruction blocks without creating memory stores or contacting providers. Lifecycle
hooks restore guidance; they do not capture transcripts or automatically write memory.

This version has no team scope, transcript capture, broad consolidation, client
delivery-acknowledgement protocol, or automatic permanent retention purge. Supporting
file/input checks run on recall; semantic symbol remapping and proactive index-event
maintenance remain follow-up work. Store commits currently retain a canonical
transactional state snapshot; large-store scaling and live recall/task accuracy need
separate measurement. Offline fixture results establish contracts, not production
performance or model quality.
