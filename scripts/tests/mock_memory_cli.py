#!/usr/bin/env python3
# ABOUTME: Stateful subprocess fixture for the CLI memory/agent runner regressions.
# ABOUTME: Emulates contracts and failures without embedding, database, or LLM providers.
import json
import os
import sys
from pathlib import Path

arguments = sys.argv[1:]
mode = os.environ.get("MOCK_MODE", "valid")
home = Path(os.environ["CODEGRAPH_MEMORY_HOME"])
home.mkdir(parents=True, exist_ok=True)
state_path = home / "mock-state.json"
state = (
    json.loads(state_path.read_text())
    if state_path.exists()
    else {
        "counter": 0,
        "claims": {},
        "jobs": {},
        "keys": {},
        "running": False,
    }
)
with Path(os.environ["MOCK_RECORD"]).open("a") as record:
    record.write(
        json.dumps(
            {
                "argv": arguments,
                "home": str(home),
                "project_id": os.environ.get("CODEGRAPH_PROJECT_ID"),
            }
        )
        + "\n"
    )


def option(name, default=None):
    return arguments[arguments.index(name) + 1] if name in arguments else default


def emit(value, code=0):
    state_path.write_text(json.dumps(state))
    print(json.dumps(value))
    sys.exit(code)


def fail(message):
    emit({"error": {"message": message}}, 1)


def identifier(prefix):
    state["counter"] += 1
    return f"{prefix}-{state['counter']}"


def context(scope):
    session = option("--session-id", os.environ.get("CODEGRAPH_MEMORY_SESSION_ID"))
    result = {
        "status": "ok",
        "limit": request.get("limit", 20),
        "token_budget": request.get("token_budget", 3000),
        "estimated_tokens": 500,
        "memories": [],
        "needs_verification": [],
        "retrieved_memory_refs": [],
        "cited_memory_refs": [],
        "warnings": [],
        "truncated": 0,
    }
    for claim in state["claims"].values():
        if claim.get("foreign"):
            continue
        if scope != "both" and claim["scope"] != scope:
            continue
        if claim["scope"] == "session" and claim["session_id"] != session:
            continue
        if claim["lifecycle"] == "archived" and not request.get("include_archived"):
            continue
        if claim["lifecycle"] == "provisional" and not request.get(
            "include_provisional"
        ):
            continue
        reference = f"memory:{claim['id']}@{claim['revision']}"
        grounded = bool(claim["anchors"]) and mode != "no-grounding"
        entry = {
            "memory": claim,
            "reference": reference,
            "reasons": claim["review_reasons"],
            "code_context": {
                "nodes": [
                    {
                        "id": "nodes:fixture",
                        "project_id": os.environ.get("CODEGRAPH_PROJECT_ID"),
                    }
                ]
                if grounded
                else [],
                "edges": [{"from": "nodes:fixture", "to": "nodes:neighbor"}]
                if grounded
                else [],
                "paths": [
                    {"code_hops": 1, "from": "nodes:fixture", "to": "nodes:neighbor"}
                ]
                if grounded
                else [],
                "snippets": [{"snapshot": "current_source", "text": "fn fixture() {}"}]
                if grounded
                else [],
                "truncated": False,
            },
        }
        section = "needs_verification" if claim["review_reasons"] else "memories"
        result[section].append(entry)
        result["retrieved_memory_refs"].append(reference)
    return result


if "memory" in arguments:
    if mode == "vanilla":
        fail("unrecognized subcommand 'memory'")
    action = arguments[arguments.index("memory") + 1]
    request = json.loads(sys.stdin.read()) if "--input" in arguments else {}
    scope = option("--scope", request.get("scope", "project"))
    request["scope"] = scope
    if action == "service":
        operation = arguments[arguments.index("service") + 1]
        if operation == "stop":
            state["running"] = False
            emit({"status": "stopped"})
        emit({"status": "running" if state["running"] else "stopped"})
    state["running"] = True
    if action == "write":
        key = request.get("idempotency_key")
        if key in state["keys"]:
            previous, job = state["keys"][key]
            if previous != request:
                fail("Idempotency key was already used with different input")
            emit(state["jobs"][job])
        claim_id, job_id = identifier("claim"), identifier("job")
        claim = {
            "id": claim_id,
            "revision": 1,
            "statement": request["statement"],
            "scope": scope,
            "session_id": option("--session-id"),
            "tier": "working" if scope == "session" else "durable",
            "kind": "fact",
            "lifecycle": "provisional",
            "evidence_state": "reported",
            "grounding": "anchored"
            if request.get("code_related")
            else "not_applicable",
            "anchors": [{"node_id": "nodes:fixture"}]
            if request.get("code_related")
            else [],
            "evidence": request.get("evidence", []),
            "review_reasons": [],
            "usefulness": 0,
            "feedback_ids": [],
        }
        job = {
            "id": job_id,
            "state": "pending",
            "embedding_ready": not request.get("asynchronous", False),
            "memory_ids": [claim_id],
            "scope": scope,
        }
        if mode == "merged-seeds" and request["statement"].startswith(
            "CLI evaluation case 4:"
        ):
            claim = next(
                claim
                for claim in state["claims"].values()
                if claim["statement"].startswith("CLI evaluation case 2:")
            )
            claim["revision"] += 1
            claim_id = claim["id"]
            job["memory_ids"] = [claim_id]
        if mode == "foreign-memory":
            state["claims"]["foreign"] = {
                "id": "foreign",
                "foreign": True,
                "statement": "Keep me.",
            }
        state["claims"][claim_id], state["jobs"][job_id] = claim, job
        state["keys"][key] = (request, job_id)
        emit(job)
    if action in ("status", "wait", "retry"):
        job = state["jobs"][arguments[arguments.index(action) + 1]]
        if action == "retry":
            fail("Only failed/configuration-blocked jobs can be retried")
        if action == "wait":
            if mode == "memory-timeout":
                import time

                print("waiting on mock classifier", flush=True)
                print("mock wait diagnostics", file=sys.stderr, flush=True)
                time.sleep(30)
            failed = mode == "classification-failed" or (
                mode == "seed-failed"
                and any(
                    state["claims"][cid]["statement"].startswith("CLI evaluation")
                    for cid in job["memory_ids"]
                )
            )
            job.update(
                state="failed" if failed else "complete", embedding_ready=True,
                error="Invalid classifier: mock proposal validation failed" if failed else None,
            )
            for claim_id in job["memory_ids"]:
                state["claims"][claim_id]["lifecycle"] = "provisional" if failed else "active"
        emit(job)
    if action == "read":
        emit(context(scope))
    if action in ("update", "delete"):
        if "--query" in arguments:
            emit({"status": "needs_selection", "candidates": context(scope)})
        claim = state["claims"].get(request.get("memory_id"))
        if not claim:
            fail("Memory not found")
        if request.get("expected_revision") != claim["revision"]:
            fail("Revision conflict")
        if action == "delete":
            if (
                mode == "cleanup-failed"
                and "investigation" in claim["statement"].lower()
            ):
                fail("mock cleanup write failure")
            del state["claims"][claim["id"]]
            for job in state["jobs"].values():
                if claim["id"] in job["memory_ids"]:
                    job.update(state="cancelled", memory_ids=[])
            emit(
                {
                    "status": "forgotten",
                    "memory_ids": [claim["id"]],
                    "source_derivatives_purged": True,
                }
            )
        if "statement" in request:
            claim.update(
                statement=request["statement"],
                revision=claim["revision"] + 1,
                lifecycle="provisional",
                review_reasons=["corrected_claim_requires_verification"],
            )
            job_id = identifier("job")
            state["jobs"][job_id] = {
                "id": job_id,
                "state": "pending",
                "memory_ids": [claim["id"]],
                "embedding_ready": True,
                "scope": scope,
            }
            emit({**claim, "operation_id": job_id})
        if request.get("confirm"):
            claim.update(
                revision=claim["revision"] + 1,
                evidence_state="verified",
                review_reasons=[],
            )
        if request.get("complete"):
            claim.update(
                revision=claim["revision"] + 1, lifecycle="archived", tier="archive"
            )
        if "useful" in request and request["feedback_id"] not in claim["feedback_ids"]:
            claim["usefulness"] += 1
            claim["feedback_ids"].append(request["feedback_id"])
        emit(claim)
    fail(f"Unhandled mock action {action}")

request = {}
memory = context("both")
if mode == "missing-memory":
    (
        memory["memories"],
        memory["needs_verification"],
        memory["retrieved_memory_refs"],
    ) = [], [], []
if mode == "wrong-revision":
    for entry in memory["memories"] + memory["needs_verification"]:
        entry["memory"]["revision"] += 100
        entry["reference"] = (
            f"memory:{entry['memory']['id']}@{entry['memory']['revision']}"
        )
    memory["retrieved_memory_refs"] = [
        entry["reference"]
        for entry in memory["memories"] + memory["needs_verification"]
    ]
if mode == "partial-memory":
    memory.update(status="partial", warnings=["Code-side projection timed out"])
memory["cited_memory_refs"] = memory["retrieved_memory_refs"][:1]
response = {
    "answer": "Mock grounded answer "
    + " ".join(f"[{ref}]" for ref in memory["cited_memory_refs"]),
    "findings": "Completed",
    "steps_taken": 3,
    "tool_use_count": 3,
    "structured_output": {"file_path": "fixture.rs", "line_number": 1},
    "memory_context": memory,
}
if mode == "no-context":
    del response["memory_context"]
emit(response)
