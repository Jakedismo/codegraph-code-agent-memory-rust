![codegraph-agent-memory](docs/assets/banner.png)

# codegraph-agent-memory

**Your codebase, understood. Your project's knowledge, remembered.**

`codegraph-agent-memory` combines a semantically searchable code graph with persistent agent memory. It brings code relationships, architectural rationale, previous findings, reusable procedures, and task history into the context an AI coding agent needs for its next change.

The code graph explains how the project fits together. Memory preserves what was learned while working on it: why a design was chosen, which approaches failed, what constraints matter, and where an interrupted task left off. A shared context service selects relevant knowledge alongside current code evidence, within the consuming model's context budget.

> **Feature status:** Persistent semantic memory is implemented. Full-feature builds provide CLI/MCP memory tools, durable background classification and reconciliation, automatic retrieval in all four agent workflows, code grounding, and project setup guidance. Automatic retrieval is opt-in. See the [memory usage guide](docs/AGENT_MEMORY.md) for configuration, readiness states, and current limits; live task quality and large-store performance remain to be evaluated.
>
> The project name is `codegraph-agent-memory`. Command examples retain the existing `codegraph` executable, `.codegraph` paths, and `CODEGRAPH_*` settings used by the design. A package or executable rename is not assumed.

- [Installation Guide](docs/INSTALLATION_GUIDE.md)
- [Usage Guide](docs/USAGE_GUIDE.md)
- [Agent CLI and project-local hooks](docs/AGENTIC_CLI.md)
- [Persistent agent memory](#persistent-agent-memory)
- [Architecture](#architecture)

## Why code context and memory belong together

An agent can find the implementation of a cache without knowing why its invalidation rules were chosen. It can identify a dependency without knowing that a previous attempt to remove it broke compatibility. After a session ends or context is compacted, even a useful investigation can disappear from the next task's context.

`codegraph-agent-memory` addresses both code discovery and continuity:

| Agent question | Code graph contributes | Memory contributes |
| --- | --- | --- |
| "How does authentication work?" | Implementations, callers, dependencies, and documentation | Relevant conventions, decisions, and previous investigations |
| "What could break if I change this?" | Call chains, reverse dependencies, and affected modules | Behavioral guarantees, compatibility constraints, and failed approaches |
| "Why is this boundary here?" | Package structure, public APIs, and boundary rules | Design rationale and rejected alternatives |
| "What should we refactor first?" | Complexity, coupling, and hotspot evidence | Known risks, deliberate tradeoffs, and prior findings |
| "Continue the interrupted task." | Current implementation and related code | Authorized working state, objective, blockers, and latest results |

Memory supplies historical knowledge with its provenance and applicability. Current code supplies evidence about the implementation being examined. When the two disagree, the system exposes the discrepancy for verification.

The intended benefit is less repeated discovery and better continuity across sessions. Accuracy, latency, and token savings require evaluation; adding memory does not establish those improvements by itself.

## Four agentic workflows

The public code-analysis interface remains four consolidated workflows. Each runs a reasoning agent that searches, follows graph relationships, and synthesizes an answer with structured evidence.

| MCP tool | CLI command | Code analysis | Memory emphasis when enabled |
| --- | --- | --- | --- |
| `agentic_context` | `codegraph agent context` | Semantic questions, code discovery, and context assembly | Local conventions, procedures, implementation decisions, and task history |
| `agentic_impact` | `codegraph agent impact` | Dependencies, call chains, and change impact | Compatibility constraints, behavioral guarantees, and previous failed changes |
| `agentic_architecture` | `codegraph agent architecture` | System structure, API surfaces, and architectural patterns | Design rationale, architectural constraints, and rejected alternatives |
| `agentic_quality` | `codegraph agent quality` | Complexity, coupling, hotspots, and refactoring priorities | Investigated findings, deliberate tradeoffs, and unresolved risks |

```bash
codegraph agent context "How does configuration reach the agent?"
codegraph agent impact "What depends on the embedding pipeline?"
codegraph agent architecture "Explain the public API and module boundaries"
codegraph agent quality "Which components need refactoring?"
```

Optional `focus` values narrow the analysis:

| Tool | Focus values | Default |
| --- | --- | --- |
| `agentic_context` | `search`, `builder`, `question` | Selected from the query |
| `agentic_impact` | `dependencies`, `call_chain` | Both |
| `agentic_architecture` | `structure`, `api_surface` | Both |
| `agentic_quality` | `complexity`, `coupling`, `hotspots` | Comprehensive assessment |

Memory-aware operation is an explicit project/client setting: configure `[memory] enabled = true`, set `CODEGRAPH_MEMORY_ENABLED=true`, or pass `--memory on` to an agent command. Once enabled, all four workflows retrieve relevant memories automatically. Their emphasis guides ranking; it does not restrict a workflow to particular memory kinds. A procedure or a requirement without a code anchor can matter to any workflow.

### How memory enters a workflow

The shared context service retrieves relevant memories and verification warnings before reasoning starts. As code discovery finds useful symbols and graph paths, bounded follow-up retrieval can add relevant knowledge before the answer is finalized.

The final result preserves the existing answer and code findings, and adds structured memory context:

- `memories`: selected claims with IDs, revisions, evidence, scope, and applicability.
- `needs_verification`: relevant conflicts or claims whose supporting evidence needs review.
- `retrieved_memory_refs`: claim revisions supplied to the workflow.
- `cited_memory_refs`: claim revisions explicitly cited by the selected answer.
- Retrieval metadata: status, triggers, selected counts, budgets, truncation, and source failures.

Retrieving a memory does not mean the answer cited it. Neither retrieval nor citation verifies the claim. Successful retrieval with no matches is also distinct from disabled, unavailable, or partial memory access.

## Persistent agent memory

The calling agent submits an observation and its available evidence. The system handles claim extraction, classification, code anchoring, semantic reconciliation, retention, and placement in future context.

An agent does not need to choose a taxonomy, storage tier, deduplication action, or default TTL before writing. It can supply explicit scope, expiry, or applicability when those boundaries are known.

### What can be remembered

| Kind | Example |
| --- | --- |
| `fact` | A finding about configuration behavior, with its supporting source or test |
| `decision` | A chosen design and the reasons alternatives were rejected |
| `preference` | An explicitly expressed working convention |
| `procedure` | Reusable steps and the conditions under which they apply |
| `episode` | An investigation, attempted change, failure, or useful outcome |
| `working_state` | A task objective, constraints, blockers, hypotheses, and latest results |

`unclassified` remains valid while processing is incomplete or uncertain. One observation can produce several atomic claims of different kinds.

Code anchors are optional. Requirements, collaboration conventions, and rationale can be useful without a direct symbol or file association. Such memories remain eligible for semantic retrieval and applicable context without fabricated code links or an automatic credibility penalty.

### Capture is deliberate

The implementation processes observations deliberately submitted by an agent or authorized client integration. Useful capture points include decisions, meaningful outcomes, corrections, explicit preferences, and task handoffs.

It does not assume access to a client's full conversation history or automatically archive every tool call. Compact outcome summaries and small evidence excerpts are in scope; bulk transcript and diff ingestion are outside the initial scope. Automatic capture from trustworthy task/session events is a later integration.

A generated answer or untested plan must retain that status when submitted. The system's own answers are not independent evidence for the memories they used.

### Scope and isolation

| Scope | Intended use | Boundary |
| --- | --- | --- |
| `session` | Working knowledge for a task or session | Requires an authorized session identity supplied by the client or caller |
| `project` | Codebase decisions, practices, findings, and outcomes | Uses a stable project identity across its branches and worktrees |
| `user` | Explicitly selected preferences and conventions across projects | Stored outside repositories under an owner identity |

Writes default to the current project. User scope must be selected explicitly or authorized by an established client policy. A classifier may suggest a different scope, but cannot widen access or move content there automatically. Promotion from session to project also requires authorization.

Branch, revision, environment, task conditions, and temporal applicability are tracked separately from access scope. Organization/team memory requires a future sharing and permission model; a shared database alone does not provide it.

### Logical memory tiers

| Tier | Typical contents | Purpose |
| --- | --- | --- |
| `working` | Objectives, blockers, hypotheses, and recent task results | Session continuity and handoff |
| `durable` | Decisions, useful findings, preferences, procedures, and outcomes | Recall across sessions |
| `core` | A small set of established constraints and proven procedures | Eligibility for applicable context packs under strict evidence, authority, and size limits |
| `archive` | Superseded claims, historical outcomes, and inactive working state | Historical explanation and verification |

These are lifecycle and serving policies, not four mandatory databases. Kind, scope, and tier are independent: a procedure need not be core, and an episode can be durable.

An LLM proposes classification and reuse potential. Deterministic policy decides scope, persistence, retention, and core eligibility. Repeated retrieval does not establish truth or promote a memory into core.

### Semantic writing and background processing

The write pipeline separates acceptance from completed processing:

1. Validate the observation, authorized scope, metadata, and provider input limits.
2. Embed the observation and persist it with a provisional retrieval representation and a recoverable background job.
3. Return an operation ID and explicit processing/embedding readiness.
4. Extract atomic claims in a bounded background LLM job, preserving uncertainty, negation, conditions, and dates.
5. Retrieve existing candidates and propose relationships; enrich code anchors during recall when the code index is available.
6. Validate the proposal and commit claims, revisions, provenance, relationships, and job completion transactionally.

Default acknowledgement requires durable acceptance and semantic visibility to the originating session once the compatible embedding/index is ready. It does not mean classification is complete. Asynchronous acceptance explicitly reports pending embeddings. Provisional observations are session-visible or explicitly requested; they do not enter shared active context before reconciliation.

Claim writing requires embeddings. Automatic extraction and reconciliation also require a configured LLM capability. Embeddings-only deployments can retain provisional observations, but must expose their processing limitation. Provider failures never silently turn a semantic operation into lexical-only storage or recall.

Background jobs persist their state and leases so interrupted work can recover after restart. Revision checks and scoped idempotency keys prevent silent overwrites and duplicate replay. Background inference has separate deadlines, retries, concurrency, and cost bounds.

### Reconciliation preserves meaning and disagreement

| Relationship | Intended behavior |
| --- | --- |
| Equivalent | Retain the claim and attach additional provenance |
| Complementary | Keep both claims and connect them, preserving conditions |
| Contradictory | Preserve the conflict unless evidence and policy resolve it |
| Explicit correction | Supersede the selected revision with an authorized replacement |
| Unrelated | Create a new claim |

Similarity proposes candidates; it does not authorize merging or supersession. Opposite statements must remain distinct, and a newer statement is not automatically more authoritative. Reconciliation mutates candidates only within the same owner and access scope. Ambiguous natural-language correction targets require disambiguation.

### Semantic recall with code context

Memory retrieval combines compatible semantic embeddings, lexical matching over claim text, and memories associated with relevant code nodes and bounded graph neighbors. Scope, lifecycle, and applicability filters apply before candidate selection.

Ranked lists are fused with reciprocal rank fusion (RRF), then assessed for relevance, evidence, conflict, and redundancy. The configured shared reranker can refine the results. Unanchored memories participate through semantic and lexical retrieval; graph proximity does not determine truth.

Results carry claim/revision IDs, provenance, evidence state, applicability, review status, and code-grounding status. Code-related results include bounded snippets and paths when the source is available:

- **Evidence paths** connect a claim to supporting code, tests, or documentation. Topically related anchors are labeled separately.
- **Retrieval paths** explain which matched nodes and graph edges led to the memory. Direct semantic or lexical matches do not invent graph edges.

Paths preserve direction, edge type, resolution provenance, and whether a relationship is exact, contextual, or inferred. Snippets identify their source snapshot. Historical evidence and current code context remain distinguishable through edits, renames, and index rebuilds.

### Verification, time, and forgetting

Processing readiness, embedding readiness, lifecycle, evidence state, and code grounding are separate signals. A fully processed memory can still be reported or disputed. A missing intended code anchor does not by itself prove a claim false.

Recall compares supporting source fingerprints and index readiness. Changed or missing supporting evidence triggers review; changes to merely related code do not invalidate a claim. Proactive index-event maintenance and semantic symbol remapping remain follow-up work.

Relevant review-required memories appear in `needs_verification`, enabled by default for memory-aware workflows. Setting `include_stale: false` excludes stale claims from ordinary results while preserving relevant verification warnings. Deliberately non-code memories do not acquire warnings simply because they lack code anchors.

The implementation separates event time, recording time, asserted validity, review deadlines, active expiry, and permanent purge policy. Historical reads can select claims valid at a requested time. Expiry stops ordinary active retrieval; it does not automatically erase history. Reading does not extend a TTL, refresh evidence, or confirm truth.

Corrections create traceable revisions. Explicit confirmation requires a declared re-verification and evidence references; metadata edits and usefulness feedback are not confirmations.

Explicit forgetting removes selected content and affected derivatives, including revisions, embeddings, summaries, source content as needed, and pending jobs. In-flight workers and caches must not restore forgotten claims. Ambiguous delete targets require disambiguation, and cross-store operations report partial completion rather than claiming a single atomic transaction.

### Direct memory tools

The MCP surface provides four tools alongside the four agentic workflows:

| MCP tool | CLI equivalent | Purpose |
| --- | --- | --- |
| `memory_write` | `codegraph memory write` | Submit observations for semantic acceptance and reconciliation |
| `memory_read` | `codegraph memory read` | Recall knowledge or request a bounded context pack |
| `memory_update` | `codegraph memory update` | Correct a claim, update metadata, or explicitly confirm evidence |
| `memory_delete` | `codegraph memory delete` | Forget precisely selected content and affected derivatives |

`codegraph memory status <operation-id>`, `wait <operation-id>`, and `retry <operation-id>` expose background progress separately from recall. `memory service status/stop` manages the local owner, and `memory reembed` explicitly migrates embedding identities. Structured requests use `--input file.json` or `--input -` for stdin. See [memory usage](docs/AGENT_MEMORY.md).

```bash
codegraph memory write "Preserve scalar summation order in semantic scoring"
codegraph memory read "What scoring constraints should I preserve?"
codegraph memory write "Prefer short explanations" --scope user
codegraph memory read "Previous scoring decisions" --limit 5 --token-budget 2000
codegraph memory update --query "scoring constraints"  # select candidates
codegraph memory update "Corrected observation" --memory-id <id> --expected-revision 1
codegraph memory delete --memory-id <id> --expected-revision 2
codegraph agent context "Explain semantic scoring" --memory on
```

Use `codegraph memory --help` or a subcommand's `--help` for available flags.
Mutation requires the exact claim ID and current revision; `--query` returns
selection candidates without modifying them. Evidence-rich requests remain available
through `--input`, with explicit CLI fields overriding matching JSON fields.

Example `memory_write` input:

```json
{
  "statement": "We rejected parallel dot-product reductions because they changed resolution decisions.",
  "evidence": [
    {
      "uri": "commit:<sha>",
      "description": "Implementation and regression evidence"
    }
  ]
}
```

Illustrative `memory_read` input:

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

`both` selects the authorized current project and user scopes. Session context is added only with an authorized session identity. Optional filters include applicability, time, kind, and symbols or node IDs; a semantic query remains required.

## Code graph foundation

The existing indexing pipeline combines structural parsing, semantic embeddings, and relationship analysis:

```text
Source code -> Build context -> AST + FastML -> LSP resolution -> Enrichment
                                                                  |
                                                                  v
                                                  Code graph + embeddings
                                                                  |
                                                                  v
                                               Search + graph reasoning
```

Depending on the indexing tier, the graph includes functions, classes, modules, imports, calls, documentation/spec links, Rust-local dataflow, package cycles, and optional architecture-boundary violations.

Code search combines semantic and lexical signals with graph traversal and optional reranking. The foundation's documented code-search blend uses 70% vector similarity and 30% lexical matching. Memory retrieval uses its own ranked-list fusion over memory candidates; those code-search weights are not memory truth scores.

### Indexing tiers

| Tier | Coverage | Typical use |
| --- | --- | --- |
| `fast` (default) | AST nodes and core edges; no LSP or enrichment | Quick indexing and lower storage |
| `balanced` | Build context, LSP symbols, enrichment, module linking, and docs/contracts | Richer navigation with moderate analyzer cost |
| `full` | All analyzers, LSP definitions, dataflow, and architecture | Maximum analyzer coverage |

`fast` filters out `Uses`/`References` edges; `balanced` filters out `References`; `full` applies no tier edge filtering. Indexing tiers are independent of memory tiers and model context-window tiers.

```bash
codegraph index /path/to/project --index-tier balanced
# Or: CODEGRAPH_INDEX_TIER=balanced
# Or: [indexing] tier = "balanced" in TOML
```

Directory indexing and estimation recurse by default. Use `--no-recursive` for a root-only scan; `-r`/`--recursive` remain accepted.

LSP-enabled tiers fail early when required external tools are missing:

| Language | Required tools |
| --- | --- |
| Rust | `rust-analyzer` |
| TypeScript / JavaScript | `node`, `typescript-language-server` |
| Python | `node`, `pyright-langserver` |
| Go | `gopls` |
| Java | `jdtls` |
| C / C++ | `clangd` |

For rustup-managed Rust projects, install the component from the target project directory with `rustup component add rust-analyzer`, then check `rust-analyzer --version`. A rustup shim alone does not guarantee a runnable language server.

Symbol and definition requests retry transient LSP `ContentModified` (`-32801`) responses with bounded backoff under one 30-second deadline. Changed document versions, exhausted retries, and other errors retain method/file diagnostics.

### Incremental indexing and embedding inputs

Full, single-file, and watch indexing share project reconciliation. Unchanged sources reuse cached extraction artifacts, while reconciliation handles edits, renames, deletions, and retained callers. `--force` rebuilds derived code state without trusting the previous catalog. Durable memory lives in a separate store and is preserved.

Embedding and semantic-resolution work have independent `sync`, `deferred`, and `off` policies:

```bash
CODEGRAPH_EMBEDDING_POLICY=deferred CODEGRAPH_SEMANTIC_RESOLUTION=off \
  codegraph index /path/to/project --index-tier fast --stats-json indexing.json
codegraph index /path/to/project --complete-deferred --stats-json completed.json
```

Deferred runs persist resumable jobs and distinguish graph readiness from pending inference.

Code chunks start from AST source spans. Units that fit the complete input budget stay intact; oversized units split at structural boundaries, with UTF-8-safe line/token fallback. Counting includes model prefixes and special tokens. Recognized models use matching publisher tokenizers; custom/offline models can supply a local tokenizer.

Ollama requests use `truncate=false` so a context mismatch fails visibly. Prepared-vector cache identities include the model, task, tokenizer/runtime, and input policy. Changes to these settings can require re-embedding even when vector dimensions are unchanged.

Memory reuses the same embedding and reranking interfaces, provider configuration, input validation, batching, and compatible runtime instances. Oversized observations and claims must not be silently truncated. The user store keeps a stable embedding identity independent of the calling project; queries are encoded per compatible space and ranked results are fused rather than comparing raw cosine scores across incompatible spaces.

See [indexing configuration and invariants](docs/indexing-performance-work.md) and [reproducible benchmarks](docs/indexing-benchmarks.md) for detailed controls.

## Reasoning and context budgets

The foundation uses the Rig framework with runtime architecture selection:

- `react` (default; `rig` alias): a tool-calling reasoning loop.
- `lats`: tree search with branch-local observations and evidence-based candidate evaluation.
- `reflexion`: ReAct with error feedback on retry.

```bash
codegraph start stdio
CODEGRAPH_AGENT_ARCHITECTURE=lats codegraph start stdio
```

All architectures serve the same four workflows. LATS adds candidate exploration and evaluator calls, which can increase latency and inference cost. Whole-workflow deadlines and shared result budgets still apply.

Default reasoning rounds depend on the configured agent context window:

| Context window | Default rounds |
| --- | --- |
| Below 50K tokens | 3 |
| 50K–150K tokens | 5 |
| 150K–500K tokens | 6 |
| Above 500K tokens | 8 |

Actual runs can finish earlier. Eight rounds is the largest default tier budget; programmatic builder overrides can differ.

The existing implementation bounds content snippets, individual tool results, and accumulated tool-result bytes:

```bash
CODEGRAPH_CONTEXT_WINDOW=128000
CODEGRAPH_TOOL_CONTENT_CHARS=2000
CODEGRAPH_AGENT_RESULT_BUDGET_BYTES=400000
```

Memory context is packed against its configured allowance using the consuming model's tokenizer, including wrappers, citations, and warnings. Unknown tokenizers use a declared conservative UTF-8 byte estimate with headroom. Retrieval calls, candidate counts, graph expansion, and elapsed time are bounded too.

Applicable mandatory constraints receive reserved space. If they cannot fit, packing returns an explicit budget error. Verification warnings take priority over optional surrounding graph detail, and repeated snippets or claim revisions are deduplicated.

Each reasoning run still has a bounded execution context. Persistent memory provides continuity across runs; the client remains responsible for its live conversation and session identity. ReAct and LATS share retrieval semantics while keeping candidate-specific discoveries local to their branches. Unverified candidate conclusions are not automatically persisted as shared evidence.

Memories retain their original authority. Learned procedures and core eligibility do not override user instructions, repository rules, or client permissions.

## Architecture

The implemented architecture adds durable memory and shared context assembly to the existing code-analysis foundation:

```text
                  Coding agent / authorized client hooks
                                  |
                              MCP or CLI
                                  |
           +----------------------+----------------------+
           |                                             |
  Four agentic workflows                         Four memory tools
  context / impact /                             write / read /
  architecture / quality                         update / delete
           |                                             |
           +-------------- Shared context service ---------+
                                  |
                     Scope + applicability + budgets
                                  |
           +----------------------+----------------------+
           |                                             |
    Code search and graph                         Memory service
    reasoning                                     recall / revisions /
           |                                      verification / retention
    Derived code index                                   |
    nodes / edges / chunks                       Durable memory stores
           |                                     observations / claims /
           |                                     provenance / jobs
           |                                             ^
           |                                             |
           |                               Background claim processing
           |                               extraction / classification /
           |                               reconciliation proposals
           |                                             |
           +---------- Shared embedding / reranking -----+
                      providers and input policies
```

The background LLM proposes classifications and relationships. Deterministic policy validates scope, authority, revisions, and permitted changes before persistence. Code links retain node IDs, source locators, and historical fingerprints, with current-index enrichment during recall; they do not depend on cross-database foreign references.

### Storage and ownership

| Data | Location | Lifecycle |
| --- | --- | --- |
| Code index | `<project>/.codegraph/db` | Derived and rebuildable |
| Project memory | `<project>/.codegraph/memory-db` | Durable claims, history, provenance, and jobs |
| User memory | `~/.codegraph/user-memory-db` | Owner-scoped knowledge outside repositories |

Session records live in the appropriate memory store with session isolation. Index rebuilds, `--force`, schema bootstrap, and code-node deletion must not erase memory. Memory has its own migrations and explicit lifecycle operations. Recreating the code index means replacing only its derived store, never the entire `.codegraph` directory.

The current embedded code store allows one owning process at a time. Stop a running project server before indexing that same store with another process, and use the server's in-process watcher for updates.

Memory CLI commands and project servers attach to a shared OS-user owner through authenticated local IPC. Private discovery files and a lifetime lock prevent competing embedded writers. Accepted jobs continue after clients exit; durable leases recover interrupted work when a compatible client reconnects with provider settings. The memory owner does not open the code index.

A shared SurrealDB server or Surreal Cloud remains an option for the code foundation. Cross-store memory operations must report each store's outcome, and server deployment does not imply organization/team memory permissions.

The existing [interactive architecture diagram](docs/architecture/codegraph-architecture.html) and [agent context flow](docs/architecture/agent-context-gathering-flow.html) document the CodeGraph foundation; their diagrams may require updates for the memory refactor.

## Quick start

These steps enable code analysis and memory. Full-feature builds include both interfaces; direct semantic memory does not require a code index.

### 1. Build

From your `codegraph-agent-memory` checkout:

```bash
cd /path/to/codegraph-agent-memory
./install-codegraph-full-features.sh
```

The supplied foundation requires Rust 1.95 or newer and uses edition 2024. `Cargo.lock` records the dependency graph. Optional macOS LLVM linker targets are `make build-llvm` and `make test-llvm`; see the [Installation Guide](docs/INSTALLATION_GUIDE.md).

### 2. Configure providers

Indexing requires an embedding provider. Agentic reasoning requires an LLM; automatic memory extraction/reconciliation also requires a configured LLM. Put settings in a project `.env` or export them before startup:

```bash
# Embeddings
CODEGRAPH_EMBEDDING_PROVIDER=ollama
CODEGRAPH_EMBEDDING_MODEL=qwen3-embedding:0.6b
CODEGRAPH_EMBEDDING_DIMENSION=1024

# Agent LLM: select a model available through your provider
CODEGRAPH_LLM_PROVIDER=anthropic
CODEGRAPH_LLM_MODEL="<your-model-id>"
CODEGRAPH_CONTEXT_WINDOW=128000             # Set your model's actual limit
ANTHROPIC_API_KEY="<your-api-key>"
```

Replace placeholders with your settings. Set the model and its actual context window explicitly for reproducible behavior. Environment variables take precedence over configuration-file LLM settings. API keys are read from the environment.

Memory reuses these providers and the configured LLM for bounded background processing. Enable automatic retrieval in your TOML configuration:

```toml
[memory]
enabled = true
token_budget = 3000
limit = 10
```

Direct memory commands work independently of this automatic-retrieval setting. User-store embedding settings are pinned independently of project vectors.

See [.env.example](.env.example) and [AI provider configuration](docs/AI_PROVIDERS.md).

### 3. Initialize and index

```bash
codegraph init /path/to/project --hooks both --index-tier balanced

# Install project guidance without indexing
codegraph init /path/to/project --hooks none --no-index

# Index directly with language filters
codegraph index /path/to/project -l rust,typescript,python
```

Initialization offers Claude Code, Codex, both, or no project-local hooks. It updates managed guidance in both `AGENTS.md` and `CLAUDE.md`, preserves unrelated instructions/settings, and leaves user-level harness configuration untouched. `--hooks none` still updates instructions and preserves existing hooks.

The managed guidance includes observation capture, recall, correction, confirmation, forgetting, and `needs_verification`. Repeated initialization is idempotent. `--no-index` installs guidance without loading providers or models, opening memory stores, or performing memory operations. Installing instructions does not itself enable transcript capture or write memories.

The code index uses embedded SurrealDB/SurrealKV with no separate server to start. Indexing respects ignore rules and configured secret-file exclusions; review what is indexed and what evidence is deliberately submitted for memory.

### 4. Connect your agent

For an MCP client:

```json
{
  "mcpServers": {
    "codegraph-agent-memory": {
      "command": "/full/path/to/codegraph",
      "args": ["start", "stdio", "--watch"]
    }
  }
}
```

Or use the CLI workflows directly. Ensure `codegraph` is on the agent harness's PATH and review/reload project hooks after setup.

### 5. Keep the index current

```bash
# Watch inside the MCP server process
codegraph start stdio --watch

# Standalone watcher when no embedded project server owns the store
codegraph daemon start /path/to/project --languages rust,typescript
```

Changes are detected, debounced, and re-indexed. A standalone daemon cannot share the current embedded project store with a running server; a shared SurrealDB server is an alternative.

Re-indexing preserves durable knowledge and historical locators. Memory recall checks supporting fingerprints against current source; proactive evidence review from indexing events remains follow-up work.

## Configuration and providers

Project settings use `./.codegraph.toml`; user settings use `~/.codegraph/config.toml`. Dotenv configuration is loaded before worker threads start. `codegraph init` sets up a project; `codegraph config init` separately creates global application configuration.

Example TOML; replace the model and context-window value with your provider's settings:

```toml
[embedding]
provider = "ollama"
model = "qwen3-embedding:0.6b"
dimension = 1024

[llm]
provider = "anthropic"
model = "your-model-id"
context_window = 128000

[indexing]
tier = "fast"
```

`codegraph config agent-status` reports the effective provider, model, and agent tier. `[llm] enabled = false` makes the agent ignore that section. Library callers should initialize their environment at process startup; `ConfigManager::load()` only reads it.

| Capability | Providers / storage |
| --- | --- |
| Local embeddings | Ollama, LM Studio, ONNX Runtime |
| Cloud embeddings | OpenAI, Jina AI |
| Local reasoning | Ollama, LM Studio |
| Cloud reasoning | Anthropic, OpenAI, xAI, OpenAI-compatible endpoints |
| Default code storage | Embedded SurrealDB 3.x with SurrealKV and HNSW indexes |
| Optional code storage | Shared SurrealDB server or Surreal Cloud through `CODEGRAPH_SURREALDB_URL` |

The schemas support embedding dimensions from 384 to 4096. Equal dimensions do not establish compatible embedding spaces. Provider/model/task changes can require migration and re-embedding, including for memory.

### Selected advanced indexing controls

| Control | Purpose |
| --- | --- |
| `--batch-size` | Maximum texts per embedding request; default 64, explicit CLI value wins |
| `CODEGRAPH_EMBEDDINGS_BATCH_SIZE` | Batch-size environment setting; singular legacy alias has lower precedence |
| `CODEGRAPH_EMBEDDING_BATCH_TOKENS`, `CODEGRAPH_EMBEDDING_BATCH_BYTES` | Independent request token/byte bounds |
| `CODEGRAPH_CHUNK_MAX_TOKENS` | Lower the complete-input chunk target within serving limits |
| `CODEGRAPH_CHUNK_SMART_SPLIT=0` | Use token splitting for oversized code units |
| `CODEGRAPH_CHUNK_OVERLAP_TOKENS` | Token overlap; default 64, zero disables |
| `CODEGRAPH_EMBEDDING_SKIP_CHUNKING=1` | Keep nodes intact and reject inputs exceeding the limit |
| `CODEGRAPH_TOKENIZER_PATH` | Matching local tokenizer for custom/offline models |
| `CODEGRAPH_TOKENIZER_REPO`, `CODEGRAPH_TOKENIZER_REVISION` | Publisher tokenizer override |
| `CODEGRAPH_MODEL_REVISION` | Declare an immutable model revision for cache identity |
| `CODEGRAPH_OLLAMA_NUM_CTX`, `CODEGRAPH_MODEL_MAX_TOKENS` | Serving context and model-context bounds |
| `CODEGRAPH_EMBEDDING_DOCUMENT_PREFIX`, `CODEGRAPH_EMBEDDING_QUERY_PREFIX` | Retrieval prefixes for custom models |
| `CODEGRAPH_VECTOR_INDEX_MODE` | `all`, `selected`, `deferred`, or `off` for fresh embedded stores |
| `CODEGRAPH_LSP_REQUESTS` | Outstanding requests per language server; default 32 |
| `CODEGRAPH_ANALYZERS=0` | Disable analyzers independently of indexing tier |
| `CODEGRAPH_SCIP_INDEX` | Compiler-index alternative to LSP, with source-validation requirements |

Jina uses model-aware passage/query task pairing; explicit supported task settings are honored. Reindex after changing task or request options. Memory applies the same validated input policies and reports unavailable or partial semantic coverage when compatible indexes are not ready.

### Optional graph schema and boundary rules

`CODEGRAPH_USE_GRAPH_SCHEMA=true` selects the experimental graph-oriented schema for a fresh store. Existing stores retain their original schema; changing schemas requires a fresh derived code store and re-indexing. Preserve the separate memory store. Server mode can select `CODEGRAPH_GRAPH_DB_DATABASE`; `CODEGRAPH_SCHEMA=v1` selects the original schema for new stores.

Optional package-boundary rules use `codegraph.boundaries.toml`:

```toml
[[deny]]
from = "your_crate"
to = "forbidden_crate"
reason = "Explain the architectural boundary"
```

Indexing emits `violates_boundary` edges for matching dependencies. A remembered rationale can complement these rules, but does not replace enforcement.

## Supported languages

Tree-sitter parsing and the foundation's analysis pipeline support:

Rust · Python · TypeScript · JavaScript · Go · Java · C++ · C · Swift · Kotlin · C# · Ruby · PHP · Dart

Analyzer depth varies by language and tier. Non-code memory does not require a language parser or code anchor.

## Evaluation and practical limits

The supplied README records October 6, 2026 CLI evaluations using eight questions across the four workflows. Command/response success was 8/8 for fast and both balanced runs, and 7/8 with one timeout for the original full run. Source review found useful answers alongside incorrect, incomplete, or unsupported conclusions. No overall factual accuracy score was assigned.

`OK` means a valid response completed under the runner's checks. It does not prove every graph operation succeeded or every statement is correct. More graph richness does not guarantee a better answer.

The refreshed balanced evaluation improved hub ranking and ratio arithmetic, but retained coverage gaps and an API case taking 587.5 seconds. It also changed embeddings, chunk policy, index contents, binary, and deadlines, and retrieved earlier evaluation material. It was an operational smoke test with source review, not a held-out or controlled tier benchmark.

Full-tier follow-up fixes bounded tool results, repaired the hub query, and corrected integer ratio division. Targeted reruns succeeded; they do not constitute a complete rerun of every case.

- [Balanced CLI evaluations and retained answers](docs/evaluations/balanced-cli-2026-10-06.md)
- [Full CLI evaluation and follow-up](docs/evaluations/full-cli-2026-10-06.md)
- [CLI evaluation runner](test_cli_agentic.py) and [shared test questions](agentic_test_cases.py)

Memory evaluation must separately measure semantic recall and precision, incorrect merges/supersession, conflict handling, staleness warnings, foreground latency, time to active readiness, context size, background backlog, and inference cost.

Coding-task comparisons should include code-only retrieval, retrieval without classification, and the full memory-aware workflow under fixed context/cost budgets. Offline fixtures establish contracts, including scope isolation, recovery, correction, and forgetting; they do not establish production accuracy or speedups.

## Memory status and follow-up work

The working feature includes separate durable stores and schema, stable project/worktree identity, owner coordination, semantic acceptance and recall, persistent background jobs, classification and reconciliation, revisions and forgetting, historical reads, evidence checks, tier policy, and integration with all four agent workflows. Init installs memory guidance in both agent instruction files.

Remaining work includes live recall/task-quality and cost evaluation, large-store scaling, proactive index-event maintenance, semantic symbol remapping, client delivery acknowledgements, broader automatic capture, and derived consolidation. Organization/team memory requires an explicit sharing and permission model. These follow-ups do not block the current project/session/user memory tools.

See [memory usage and implementation boundaries](docs/AGENT_MEMORY.md) and the [architecture design](docs/architecture/agent-memory-design.md).

## Philosophy

AI coding assistants work better when they can inspect relationships in the code and recover the knowledge produced by earlier work. `codegraph-agent-memory` is designed to provide both, with bounded context, traceable sources, explicit uncertainty, and controlled memory scope.

The agent still needs to inspect current source, verify important claims, and test its changes. Persistent knowledge helps it ask better questions and carry useful learning into the next task.

**Understand the code. Preserve the reasoning. Verify what changed.**

## License

MIT

## Links

- [Installation Guide](docs/INSTALLATION_GUIDE.md)
- [Usage Guide](docs/USAGE_GUIDE.md)
- [Agent CLI and hooks](docs/AGENTIC_CLI.md)
- [Semantic memory usage](docs/AGENT_MEMORY.md)
- [AI provider configuration](docs/AI_PROVIDERS.md)
- [SurrealDB Cloud](https://surrealdb.com/cloud)
- [Jina AI](https://jina.ai)
- [Ollama](https://ollama.com)

![codegraph-agent-memory](docs/assets/footer.png)
