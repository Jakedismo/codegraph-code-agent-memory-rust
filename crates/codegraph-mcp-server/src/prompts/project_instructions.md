# codegraph

CodeGraph is a command-line interface (CLI) tool available as the `codegraph`
executable on PATH. Run every `codegraph ...` command below through your Bash/shell
execution tool, just as you would run `git` or `cargo`. For example:

```bash
codegraph agent context "Find the implementation and callers for <task>" --focus search
```

When `codegraph` is available, start code exploration with its agent tools. Ask a
specific question about the task, relevant symbols or paths instead of starting
with broad grep/rg searches:

- `codegraph agent context "Find the implementation and callers for <task>" --focus search`
  locates code; use `--focus builder` to gather implementation context or `question`
  to explain behavior.
- `codegraph agent impact "What depends on <symbol> and what would <change> affect?"`
  checks dependencies before editing; `--focus call_chain` follows call flows.
- `codegraph agent architecture "Describe <area> and its interfaces"`
  maps structure; `--focus api_surface` inspects public interfaces.
- `codegraph agent quality "Assess coupling, complexity and risks in <area>"`
  supports refactoring decisions and targeted follow-up checks.

Run from the indexed project root, or append `--project /path/to/project`; retain
the indexed `--project-id` if one was configured. Prefer the default JSON output:
inspect source locations, findings and partial-result warnings, then read the
specific files/lines before editing. Reuse useful findings and narrow follow-up
questions rather than repeating broad queries. After changes, verify against
current source and run relevant tests; the index may lag uncommitted work.

If CodeGraph is unavailable, the project is not indexed, a command fails or returns
insufficient evidence, fall back to targeted source reads and rg/grep. Known file
locations and exact-string verification also warrant direct reads/searches.
Do not install CodeGraph, download models or reindex solely to satisfy these
instructions. Agent queries use the configured model and may incur provider costs.
Use the four public agent commands; internal graph tools belong to CodeGraph's
built-in agents. Reload usage details with `codegraph agent instructions`.

Agent memory is available in builds with `memory` (included in `full`). Embeddings
are required for semantic reads/writes; background claim reconciliation also needs
the configured LLM. Automatic memory context is opt-in: set `[memory] enabled = true`
in project configuration or use `codegraph agent context "<task>" --memory on`.
All four agent workflows then retrieve relevant memories before reasoning and
after code discoveries. Inspect their `memory_context.needs_verification` before
relying on changed evidence or disputed claims.

Use MCP `memory_write`, `memory_read`, `memory_update`, and `memory_delete`, or
equivalent CLI commands:

- `codegraph memory write "<decision, conditions, outcome, and evidence>"`
  submits an observation; CodeGraph assigns kinds/tiers and reconciles it.
- `codegraph memory read "<meaning or constraints to recall>"` searches semantically.
- `codegraph memory update --input correction.json` corrects or explicitly confirms
  a selected memory with its `memory_id`, `expected_revision`, and fresh evidence.
- `codegraph memory delete --input forgetting.json` forgets a selected memory;
  a semantic query first returns candidates for disambiguation.
- `codegraph memory status <operation_id>` or `wait <operation_id>` distinguishes
  durable acceptance, embedding readiness, and completed reconciliation.

Write at meaningful decisions, verified outcomes, corrections, and handoffs.
Unanchored/non-code memories are valid; related code can carry snippets and graph
paths. Generated answers and untested plans are not verified evidence. Project
scope is the default; choose `--scope user` explicitly for cross-project memories,
or `--scope session --session-id <id>` for working state. Supply the same session
ID for later recall and use `--context-epoch <id>` after a context reset. Do not
choose tiers or manually repeat automatic retrieval unless extra recall is useful.
Pending/provisional writes are not completed shared knowledge. Initialization
installs this guidance without enabling capture or loading memory providers.

Project root: {{CODEGRAPH_PROJECT_ROOT}}. Retain a configured code graph project ID
when querying it. The durable memory project UUID is created on first memory use;
Git worktrees share the main checkout's memory store without changing graph IDs.
