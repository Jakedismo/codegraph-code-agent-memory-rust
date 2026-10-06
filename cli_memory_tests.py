# ABOUTME: Exercises semantic memory CRUD before seeding the public agent CLI evaluations.
# ABOUTME: Records exact operations/revisions and cleans only this run's private-owner claims.
"""Memory checks for test_cli_agentic.py; no commands run at import time."""

import hashlib
import json
import os
import shlex
import subprocess
import time
from datetime import datetime, timezone

# These are investigation notes, not expected agent answers. Evidence points to the
# current implementation, including the replacement for the old PromptSelector.
SEEDS = {
    1: (
        "crates/codegraph-core/src/config_manager.rs",
        "ConfigManager",
        "Configuration loading investigation: ConfigManager separates environment initialization from configuration loading.",
    ),
    2: (
        "crates/codegraph-mcp-rig/src/prompts/tier_prompts.rs",
        "get_tier_system_prompt",
        "Tier-aware prompt selection investigation: get_tier_system_prompt builds prompts using the analysis type and context tier.",
    ),
    3: (
        "crates/codegraph-mcp-tools/src/graph_tool_executor.rs",
        "GraphToolExecutor",
        "LRU cache investigation: GraphToolExecutor caches graph tool results using function names and parameters.",
    ),
    4: (
        "crates/codegraph-mcp-rig/src/prompts/tier_prompts.rs",
        "get_tier_system_prompt",
        "PromptSelector dependency investigation: the current tier prompt entry point is get_tier_system_prompt; verify historical symbol names against current source.",
    ),
    5: (
        "crates/codegraph-mcp-server/src/official_server.rs",
        # execute_agentic_workflow also has a feature-disabled stub in this file.
        # Use its unique callee so this seed does not require arbitrary disambiguation.
        "create_progress_callback_with_message",
        "Call-chain investigation: execute_agentic_workflow wires create_progress_callback_with_message into ProgressNotifier for graph analysis progress reporting.",
    ),
    6: (
        "crates/codegraph-mcp-server/src/official_server.rs",
        "CodeGraphMCPServer",
        "MCP server architecture and coupling investigation: CodeGraphMCPServer hosts the public workflows and their backend integration.",
    ),
    7: (
        "crates/codegraph-mcp-tools/src/graph_tool_executor.rs",
        "GraphToolExecutor",
        "Public API investigation: GraphToolExecutor is the executor connecting agent graph tool calls to graph functions.",
    ),
    8: (
        "crates/codegraph-mcp-server/src/official_server.rs",
        "execute_agentic_workflow",
        "Complexity hotspot and risk-score investigation: execute_agentic_workflow is a workflow orchestration function to examine with current graph metrics.",
    ),
}


class MemoryFailure(Exception):
    """A saved command or assertion failed; dependent stages must not run."""


def entries(context):
    if not isinstance(context, dict):
        return []
    return [
        entry
        for key in ("memories", "needs_verification")
        for entry in (context.get(key) if isinstance(context.get(key), list) else [])
        if isinstance(entry, dict) and isinstance(entry.get("memory"), dict)
    ]


def memory_check(context, expected_refs, require_grounding=False, require_review=False):
    """Check delivered identities, not whether an LLM merely mentions the seed text."""
    problems = []
    if not isinstance(context, dict):
        return {"status": "FAIL", "errors": ["Missing memory_context object."]}
    for key in (
        "memories",
        "needs_verification",
        "retrieved_memory_refs",
        "cited_memory_refs",
    ):
        if not isinstance(context.get(key), list):
            return {
                "status": "FAIL",
                "errors": [f"Invalid memory_context.{key}; expected an array."],
            }
    if len(entries(context)) != len(context["memories"]) + len(
        context["needs_verification"]
    ):
        return {"status": "FAIL", "errors": ["Invalid delivered memory entry."]}
    if any(
        not isinstance(ref, str)
        for ref in context["retrieved_memory_refs"] + context["cited_memory_refs"]
    ):
        return {
            "status": "FAIL",
            "errors": ["Invalid memory reference; expected a string."],
        }
    for key in ("limit", "token_budget", "estimated_tokens"):
        if not isinstance(context.get(key), int) or context[key] < 0:
            return {"status": "FAIL", "errors": [f"Invalid memory_context.{key}."]}
    delivered = entries(context)
    refs = context.get("retrieved_memory_refs", [])
    cited = context.get("cited_memory_refs", [])
    matched = [entry for entry in delivered if entry.get("reference") in expected_refs]
    if context.get("status") not in ("ok", "partial"):
        problems.append(f"Memory status is {context.get('status')!r}.")
    if context.get("status") == "partial":
        problems.append("Memory retrieval is partial; inspect its warnings.")
    if not matched:
        problems.append("None of this case's seeded memory revisions were returned.")
    actual_refs = []
    for entry in delivered:
        claim = entry.get("memory", {})
        reference = f"memory:{claim.get('id')}@{claim.get('revision')}"
        actual_refs.append(reference)
        if entry.get("reference") != reference:
            problems.append(
                "Memory reference does not identify the delivered revision."
            )
    if sorted(refs) != sorted(actual_refs):
        problems.append("Retrieved references do not match the delivered memories.")
    if not set(cited).issubset(refs):
        problems.append("Cited references include an undelivered revision.")
    if len(delivered) > context.get("limit", 0):
        problems.append("Delivered memories exceed the declared limit.")
    if context.get("estimated_tokens", 0) > context.get("token_budget", 0):
        problems.append("Memory context exceeds its declared token budget.")
    review_refs = {
        entry.get("reference") for entry in context.get("needs_verification", [])
    }
    if require_review and not any(
        entry.get("reference") in review_refs for entry in matched
    ):
        problems.append("The corrected seed was not returned in needs_verification.")
    observations = []
    for entry in matched:
        code = entry.get("code_context", {})
        if not isinstance(code, dict) or any(
            not isinstance(code.get(key), list)
            for key in ("nodes", "edges", "paths", "snippets")
        ):
            problems.append("Invalid memory code context.")
            continue
        nodes, paths, snippets = (
            code.get(key, []) for key in ("nodes", "paths", "snippets")
        )
        if require_grounding and not (
            entry["memory"].get("anchors")
            and nodes
            and paths
            and any(
                isinstance(snippet, dict)
                and snippet.get("snapshot") == "current_source"
                for snippet in snippets
            )
        ):
            problems.append(
                "Seed lacks anchors, code nodes, hop paths, or current-source snippets."
            )
        observations.append(
            {
                "reference": entry.get("reference"),
                "section": "needs_verification"
                if entry.get("reference") in review_refs
                else "memories",
                "statement": entry["memory"].get("statement"),
                "grounding": entry["memory"].get("grounding"),
                "nodes": len(nodes),
                "edges": len(code.get("edges", [])),
                "paths": paths,
                "snippets": len(snippets),
                "truncated": code.get("truncated", False),
            }
        )
    return {
        "status": "FAIL" if problems else "OK",
        "errors": problems,
        "expected_refs": sorted(expected_refs),
        "retrieved_refs": refs,
        "cited_refs": cited,
        "matched": observations,
        "warnings": context.get("warnings", []),
        "truncated": context.get("truncated", 0),
    }


class MemoryHarness:
    def __init__(self, args, binary, directory):
        self.args, self.binary, self.directory = args, binary, directory
        self.home = directory / "memory-home"
        self.session = f"cli-test-{directory.name}"
        self.env = dict(
            os.environ,
            CODEGRAPH_MEMORY_HOME=str(self.home),
            CODEGRAPH_MEMORY_SESSION_ID=self.session,
            CODEGRAPH_MEMORY_CONTEXT_EPOCH=self.session,
        )
        if args.project_id is not None:
            # Memory subcommands use the existing environment override for code ID.
            self.env["CODEGRAPH_PROJECT_ID"] = args.project_id
        self.results = []
        self.operations = {}
        self.seeds = {}

    def command(self, action, *arguments, scope="project", session=None):
        command = [self.binary]
        if self.args.config:
            command += ["--config", str(self.args.config)]
        if self.args.verbose:
            command += ["--verbose"]
        command += ["memory", action, *map(str, arguments)]
        if action != "service":
            command += [
                "--project",
                str(self.args.project),
                "--session-id",
                session or self.session,
                "--context-epoch",
                self.session,
                "--scope",
                scope,
                "--timeout-secs",
                str(self.args.memory_timeout_secs),
            ]
        return command

    def save(self, result):
        stem = f"memory_{len(self.results) + 1:03}_{result['test']}"
        result["json_file"], result["log_file"] = f"{stem}.json", f"{stem}.log"
        self.results.append(result)
        (self.directory / result["json_file"]).write_text(
            json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        (self.directory / result["log_file"]).write_text(
            f"Command: {shlex.join(result['command'])}\n"
            f"CODEGRAPH_MEMORY_HOME: {self.home}\n"
            f"Status: {result['status']} | Duration: {result['duration']:.3f}s\n"
            f"INPUT JSON:\n{json.dumps(result.get('input'), indent=2)}\n"
            f"STDOUT:\n{result['stdout']}\nSTDERR:\n{result['stderr']}\n"
            f"ASSERTION ERROR:\n{result['error'] or '(none)'}\n",
            encoding="utf-8",
        )
        print(
            f"Memory {result['test']}: {result['status']} ({result['duration']:.1f}s)",
            flush=True,
        )
        if result["error"]:
            print(result["error"], flush=True)
        if not self.args.summary_only:
            print(
                json.dumps(result.get("response"), indent=2, ensure_ascii=False),
                flush=True,
            )

    def call(
        self,
        label,
        action,
        *arguments,
        request=None,
        check=None,
        expected_error=None,
        scope="project",
        session=None,
        allow_operation_failure=False,
    ):
        command = self.command(action, *arguments, scope=scope, session=session)
        if request is not None:
            command += ["--input", "-"]
        result = {
            "test": label,
            "command": command,
            "input": request,
            "status": "ERROR",
            "returncode": None,
            "response": None,
            "stdout": "",
            "stderr": "",
            "error": None,
        }
        started = time.monotonic()
        try:
            process = subprocess.run(
                command,
                input=json.dumps(request) if request is not None else "",
                env=self.env,
                cwd=self.args.project,
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                check=False,
                timeout=self.args.memory_timeout_secs + 5,
            )
            result.update(
                returncode=process.returncode,
                stdout=process.stdout,
                stderr=process.stderr,
            )
            try:
                response = json.loads(process.stdout)
            except json.JSONDecodeError as error:
                raise MemoryFailure(
                    f"Expected one JSON response; CLI exited {process.returncode}. "
                    f"{process.stderr.strip() or error}"
                ) from error
            require(isinstance(response, dict), "Expected a single JSON object.")
            result["response"] = response
            error = response.get("error")
            if expected_error is not None:
                require(
                    process.returncode != 0
                    and isinstance(error, dict)
                    and expected_error.lower() in str(error.get("message", "")).lower(),
                    f"Expected rejection containing {expected_error!r}; received {response}.",
                )
            else:
                stored_failure = (
                    allow_operation_failure
                    and action == "status"
                    and isinstance(error, str)
                    and bool(response.get("id"))
                    and response.get("state") in ("failed", "config_required")
                )
                require(
                    process.returncode == 0 and (error is None or stored_failure),
                    str(error or process.stderr or f"CLI exited {process.returncode}"),
                )
                if check:
                    check(response)
            result["status"] = "OK"
        except subprocess.TimeoutExpired as error:
            result["status"] = "TIMEOUT"
            for key in ("stdout", "stderr"):
                value = getattr(error, key) or ""
                result[key] = (
                    value.decode("utf-8", errors="replace")
                    if isinstance(value, bytes)
                    else value
                )
            result["error"] = "Memory command exceeded its process deadline."
        except (OSError, ValueError, MemoryFailure, KeyError, TypeError) as error:
            result["error"] = str(error)
        result["duration"] = time.monotonic() - started
        self.save(result)
        if result["status"] != "OK":
            raise MemoryFailure(f"{label}: {result['error']}")
        return result["response"]

    def checkpoint(self):
        (self.directory / "memory_manifest.json").write_text(
            json.dumps(
                {
                    "memory_home": str(self.home),
                    "session_id": self.session,
                    "operations": self.operations,
                    "seeds": self.seeds,
                },
                indent=2,
                ensure_ascii=False,
            )
            + "\n",
            encoding="utf-8",
        )

    def fail_last(self, error):
        if not self.results:
            return
        result = self.results[-1]
        if result["status"] == "OK":
            result.update(status="ASSERTION_FAILED", error=str(error))
            (self.directory / result["json_file"]).write_text(
                json.dumps(result, indent=2, ensure_ascii=False) + "\n",
                encoding="utf-8",
            )
            with (self.directory / result["log_file"]).open(
                "a", encoding="utf-8"
            ) as log:
                log.write(f"\nPOST-RESPONSE ASSERTION FAILED: {error}\n")

    def write(self, label, statement, scope="project", asynchronous=False, **fields):
        request = {
            "statement": statement,
            "scope": scope,
            "idempotency_key": f"{self.session}:{label}",
            **fields,
        }
        if asynchronous:
            request["asynchronous"] = True
        accepted = self.call(
            label,
            "write",
            request=request,
            scope=scope,
            check=lambda r: require(
                bool(r.get("id")) and bool(r.get("memory_ids")),
                "Write did not return operation/claim identities.",
            ),
        )
        self.operations[accepted["id"]] = {
            "scope": scope,
            "query": statement,
            "memory_ids": accepted["memory_ids"],
            "forgotten": [],
        }
        self.checkpoint()  # Retain cleanup identities even if waiting/classification fails.
        require(
            accepted.get("embedding_ready") is (not asynchronous),
            "Write acceptance did not match the requested semantic embedding mode.",
        )
        self.call(
            f"{label}_status",
            "status",
            accepted["id"],
            scope=scope,
            check=lambda r: require(
                r.get("id") == accepted["id"], "Status returned another operation."
            ),
        )
        completed = self.call(
            f"{label}_wait",
            "wait",
            accepted["id"],
            scope=scope,
            check=completed_operation,
        )
        self.operations[accepted["id"]]["memory_ids"] = completed["memory_ids"]
        self.checkpoint()
        return accepted, completed, request

    def read(self, label, query, scope="project", **fields):
        return self.call(
            label,
            "read",
            request={
                "query": query,
                "limit": 20,
                "token_budget": self.args.memory_token_budget,
                **fields,
            },
            scope=scope,
        )

    def recall(self, label, query, ids, scope="project", **fields):
        context = self.read(label, query, scope=scope, **fields)
        matching = [
            entry
            for entry in entries(context)
            if entry.get("memory", {}).get("id") in ids
        ]
        require(
            matching, f"{label}: semantic recall did not return a claim from {ids}."
        )
        require(
            all(
                entry["memory"].get("lifecycle") in ("active", "archived")
                and entry["memory"].get("kind") not in (None, "unclassified")
                and entry["memory"].get("tier")
                in ("working", "durable", "core", "archive")
                for entry in matching
            ),
            f"{label}: completed operations did not return classified claims with policy-assigned tiers.",
        )
        check = memory_check(context, [entry["reference"] for entry in matching])
        require(check["status"] == "OK", f"{label}: {check['errors']}")
        return matching, context

    def update(self, label, claim, **fields):
        return self.call(
            label,
            "update",
            request={
                "memory_id": claim["id"],
                "expected_revision": claim["revision"],
                **fields,
            },
            scope=claim["scope"],
        )

    def corrected(self, label, claim, statement):
        corrected = self.update(
            label, claim, statement=statement, evidence=claim.get("evidence", [])
        )
        require(
            corrected["id"] == claim["id"]
            and corrected["revision"] > claim["revision"],
            "Correction did not preserve identity and advance the revision.",
        )
        operation = corrected["operation_id"]
        self.operations[operation] = {
            "scope": claim["scope"],
            "query": statement,
            "memory_ids": [claim["id"]],
            "forgotten": [],
        }
        self.checkpoint()
        self.call(
            f"{label}_wait",
            "wait",
            operation,
            scope=claim["scope"],
            check=completed_operation,
        )
        return self.recall(
            f"{label}_recall", statement, [claim["id"]], scope=claim["scope"]
        )[0][0]

    def forget(self, label, claim):
        result = self.call(
            label,
            "delete",
            request={
                "memory_id": claim["id"],
                "expected_revision": claim["revision"],
            },
            scope=claim["scope"],
            check=lambda r: require(
                r.get("status") == "forgotten"
                and claim["id"] in r.get("memory_ids", [])
                and r.get("source_derivatives_purged") is True,
                "Delete did not confirm complete forgetting.",
            ),
        )
        for operation in self.operations.values():
            if claim["id"] in operation["memory_ids"]:
                operation["forgotten"].append(claim["id"])
        self.checkpoint()
        return result

    def suite(self):
        self.call(
            "service_status",
            "service",
            "status",
            check=lambda r: require(
                r.get("status") == "stopped",
                "The fresh private memory owner was already running.",
            ),
        )
        statement = (
            "For this CLI memory test, the preferred scratch report color is amber."
        )
        accepted, completed, request = self.write("unanchored_write", statement)
        replay = self.call("idempotent_write", "write", request=request)
        require(
            replay.get("id") == accepted["id"],
            "Idempotent write created another operation.",
        )
        self.call(
            "idempotency_conflict",
            "write",
            request={**request, "statement": "A different observation."},
            expected_error="Idempotency key",
        )
        matched, _ = self.recall(
            "semantic_unanchored_read",
            "Which hue is preferred for the scratch report?",
            completed["memory_ids"],
        )
        claim = matched[0]["memory"]
        require(
            claim["grounding"] == "not_applicable" and not claim["anchors"],
            "Non-code memory must remain retrievable without code anchors.",
        )
        original_revision = claim["revision"]
        for action in ("update", "delete"):
            self.call(
                f"{action}_semantic_selection",
                action,
                "--query",
                statement,
                check=lambda r: require(
                    r.get("status") == "needs_selection"
                    and any(
                        entry["memory"]["id"] == claim["id"]
                        for entry in entries(r.get("candidates"))
                    ),
                    "Semantic mutation must select candidates without changing them.",
                ),
            )
        unchanged, _ = self.recall("selection_did_not_mutate", statement, [claim["id"]])
        require(
            unchanged[0]["memory"]["revision"] == original_revision,
            "Semantic selection mutated memory.",
        )
        corrected = self.corrected(
            "correction",
            claim,
            "For this CLI memory test, the preferred scratch report color is cobalt.",
        )
        require(
            corrected["reasons"]
            and "corrected_claim_requires_verification" in corrected["reasons"],
            "Corrected claims must require verification.",
        )
        for action in ("update", "delete"):
            self.call(
                f"{action}_stale_revision",
                action,
                request={
                    "memory_id": claim["id"],
                    "expected_revision": original_revision,
                    **({"statement": statement} if action == "update" else {}),
                },
                expected_error="Revision conflict",
            )
        verified = self.update(
            "confirm_evidence",
            corrected["memory"],
            confirm=True,
            evidence=[
                {
                    "uri": f"test:cli-memory:{self.session}",
                    "description": "Synthetic scratch preference verified by this test.",
                    "verified_at": datetime.now(timezone.utc).isoformat(),
                }
            ],
        )
        require(
            verified["evidence_state"] == "verified",
            "Confirmation did not verify the scratch claim.",
        )
        for label in ("feedback", "idempotent_feedback"):
            feedback = self.update(
                label, verified, useful=True, feedback_id=f"{self.session}:useful"
            )
            require(
                feedback["usefulness"] == verified["usefulness"] + 1,
                "Idempotent feedback changed usefulness more than once.",
            )
        self.call(
            "retry_completed_rejected",
            "retry",
            accepted["id"],
            expected_error="Only failed/configuration-blocked",
        )
        self.call("owner_stop", "service", "stop")
        self.wait_stopped()
        self.recall("persistent_recall_after_restart", statement, [claim["id"]])
        self.forget("delete_exact_revision", feedback)
        forgotten = self.read(
            "forgotten_not_recalled",
            statement,
            include_archived=True,
            include_expired=True,
            include_stale=True,
            include_provisional=True,
        )
        require(
            not any(
                entry["memory"]["id"] == claim["id"] for entry in entries(forgotten)
            ),
            "Forgotten claim is still retrievable.",
        )
        replay = self.call("forgotten_write_replay", "write", request=request)
        require(
            replay.get("state") == "cancelled" and not replay.get("memory_ids"),
            "Idempotent replay restored forgotten memory.",
        )
        _, session_job, _ = self.write(
            "session_async_write",
            "This CLI session is investigating a temporary scratch task.",
            scope="session",
            asynchronous=True,
        )
        session_entries, _ = self.recall(
            "session_recall",
            "What temporary investigation is this session working on?",
            session_job["memory_ids"],
            scope="session",
        )
        self.call(
            "other_session_isolation",
            "read",
            session=f"{self.session}-other",
            request={
                "query": "temporary scratch task",
                "token_budget": self.args.memory_token_budget,
            },
            scope="session",
            check=lambda r: require(
                not entries(r), "Another session's memories leaked."
            ),
        )
        archived = self.update(
            "complete_session", session_entries[0]["memory"], complete=True
        )
        require(
            archived["tier"] == "archive" and archived["lifecycle"] == "archived",
            "Completion did not archive.",
        )
        hidden = self.read("archive_hidden", "temporary scratch task", scope="session")
        require(
            not any(
                entry["memory"]["id"] == archived["id"] for entry in entries(hidden)
            ),
            "Archive leaked into default recall.",
        )
        self.recall(
            "archive_explicit_read",
            "temporary scratch task",
            [archived["id"]],
            scope="session",
            include_archived=True,
        )
        _, user_job, _ = self.write(
            "user_write",
            "For CLI memory tests, the preferred scratch note format is plain text.",
            scope="user",
        )
        self.recall(
            "user_semantic_read",
            "How should the scratch notes be formatted?",
            user_job["memory_ids"],
            scope="user",
        )
        project = self.read("project_excludes_user", "scratch note format")
        require(
            not any(
                entry["memory"]["id"] in user_job["memory_ids"]
                for entry in entries(project)
            ),
            "User scope leaked into project-only recall.",
        )
        self.recall(
            "both_includes_user",
            "scratch note format",
            user_job["memory_ids"],
            scope="both",
        )
        self.cleanup()

    def wait_stopped(self):
        deadline = time.monotonic() + min(self.args.memory_timeout_secs, 10)
        while True:
            status = self.call("owner_stopped", "service", "status")
            if status.get("status") == "stopped":
                return
            require(time.monotonic() < deadline, "Memory owner did not stop.")
            time.sleep(0.1)

    def seed(self, cases):
        seed_ids = {}
        for number, case in cases:
            path, symbol, note = SEEDS[number]
            source = self.args.project / path
            require(
                source.is_file(),
                f"Seed source is missing: {source}; these questions target the CodeGraph repository.",
            )
            require(
                symbol in source.read_text(encoding="utf-8"),
                f"Seed symbol is absent from current source: {symbol}.",
            )
            evidence = {
                "uri": f"file:{path}",
                "description": "Source inspected by the CLI memory evaluation harness.",
                "fingerprint": f"sha256:{hashlib.sha256(source.read_bytes()).hexdigest()}",
            }
            _, completed, _ = self.write(
                f"seed_{number:02}",
                f"CLI evaluation case {number}: {note}",
                code_related=True,
                evidence=[evidence],
            )
            matched, _ = self.recall(
                f"seed_{number:02}_probe", case[1], completed["memory_ids"]
            )
            seed_ids[number] = [entry["memory"]["id"] for entry in matched]
            self.seeds[number] = {"source": path, "symbol": symbol}
            self.checkpoint()

        # Reconciliation may merge equivalent observations and revise earlier seeds.
        # Finish all mutations before pinning the references used by agent checks.
        number, case = cases[0]
        matched, _ = self.recall(
            f"seed_{number:02}_before_review", case[1], seed_ids[number]
        )
        self.corrected(
            f"seed_{number:02}_review",
            matched[0]["memory"],
            f"Refreshed investigation note: {matched[0]['memory']['statement']}",
        )
        for index, (number, case) in enumerate(cases):
            matched, context = self.recall(
                f"seed_{number:02}_ready", case[1], seed_ids[number]
            )
            refs = [entry["reference"] for entry in matched]
            check = memory_check(
                context, refs, require_grounding=True, require_review=index == 0
            )
            self.seeds[number].update(
                {
                    "expected_refs": refs,
                    "require_review": index == 0,
                    "probe": check,
                }
            )
            self.checkpoint()
            require(
                check["status"] == "OK", f"Seed {number} not ready: {check['errors']}"
            )

    def cleanup(self):
        errors = []
        for operation_id, tracked in list(self.operations.items()):
            if set(tracked["memory_ids"]).issubset(tracked["forgotten"]):
                continue
            try:
                operation = self.call(
                    "cleanup_status", "status", operation_id, scope=tracked["scope"],
                    allow_operation_failure=True,
                )
                ids = set(tracked["memory_ids"]) | set(operation.get("memory_ids", []))
                ids -= set(tracked["forgotten"])
                if not ids:
                    continue
                context = self.read(
                    "cleanup_recall",
                    tracked["query"],
                    scope=tracked["scope"],
                    include_provisional=True,
                    include_archived=True,
                    include_expired=True,
                    include_stale=True,
                    operation_id=operation_id,
                )
                found = {
                    entry["memory"]["id"]: entry["memory"] for entry in entries(context)
                }
                for memory_id in ids:
                    # A classifier may replace provisional IDs; only its current IDs survive.
                    if memory_id in found:
                        self.forget("cleanup_delete", found[memory_id])
                    elif memory_id in operation.get("memory_ids", []):
                        errors.append(
                            f"Cleanup could not recall current claim {memory_id} of {operation_id}."
                        )
            except MemoryFailure as error:
                errors.append(str(error))
        require(not errors, "; ".join(errors))


def require(condition, message):
    if not condition:
        raise MemoryFailure(message)


def completed_operation(response):
    require(
        response.get("state") == "complete"
        and response.get("embedding_ready") is True
        and bool(response.get("memory_ids")),
        f"Background processing did not complete with semantic claims: {response}",
    )


def preview(args, cases, binary):
    """Explain dependent IDs without inventing executable values or writing input files."""
    print(
        "FIRST: memory write/read/update/delete contract suite, status/wait, idempotency, scopes, owner restart."
    )
    print(
        "THEN: seed selected questions, wait for classification, check semantic recall and code grounding."
    )
    for number, _ in cases:
        path, symbol, note = SEEDS[number]
        print(f"Memory seed {number:02}: {note}\nEvidence: {path} ({symbol})")
        print(
            shlex.join(
                [
                    binary,
                    "memory",
                    "write",
                    note,
                    "--code-related",
                    "--project",
                    str(args.project),
                ]
            )
        )
    print(
        "FINALLY: inspect returned memory_context references and clean only this run's claims."
    )
